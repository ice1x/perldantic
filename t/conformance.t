use v5.36;
use Test2::V0;
no warnings 'experimental::builtin';
use FindBin;
use lib "$FindBin::Bin/lib";

# Replays the recorded pydantic-core cases (tests/conformance/README.md) through the Perl
# library: Perldantic::FFI and the wire codec. Dicts are sent in their recorded order (as
# Perldantic::Wire::ordered), so outputs and errors can be compared exactly. Cases the core does
# not support yet, that need values Perl cannot hold, or that depend on a documented divergence
# are skipped and counted, as in the Rust runner (crates/perldantic-core/tests/conformance.rs).

use B ();
use Encode ();
use File::Find ();
use Scalar::Util qw(blessed);

use Perldantic::FFI;
use Perldantic::Test::CaseJSON qw(parse_json interpret node_get node_has);
use Perldantic::Wire;

my $root = "$FindBin::Bin/../tests/conformance/upstream";

sub skip_case ($reason) { die Perldantic::Test::CaseJSON::Skip->new($reason) }

# ---- raw node helpers ----------------------------------------------------------------------

sub pairs_of ($node) { ref $node eq 'Perldantic::Wire::Ordered' ? @$node : () }

# Calls $visit->(\@pairs) for every object in a raw node; stops when it returns true.
sub any_object ($node, $visit) {
    if (ref $node eq 'ARRAY') {
        for (@$node) { return 1 if any_object($_, $visit) }
        return 0;
    }
    return 0 if ref $node ne 'Perldantic::Wire::Ordered';
    my @pairs = @$node;
    return 1 if $visit->(\@pairs);
    for (my $i = 1; $i < @pairs; $i += 2) { return 1 if any_object($pairs[$i], $visit) }
    return 0;
}

sub tag_of ($pairs) { @$pairs == 2 && $pairs->[0] =~ /\A\$/ ? $pairs->[0] : '' }

sub classes ($schema) {
    my @classes;
    any_object($schema, sub ($p) { push @classes, $p->[1] if tag_of($p) eq '$class'; 0 });
    return @classes;
}

sub has_foreign_model ($input, $classes) {
    return any_object($input, sub ($p) {
        return 0 if tag_of($p) ne '$model';
        my $class = node_get($p->[1], 'class');
        return !grep { $_ eq $class } @$classes;
    });
}

sub divergence ($case) {
    my ($schema, $config) = (node_get($case, 'schema'), node_get($case, 'config'));
    my $python_re = sub ($p) {
        my %h = @$p;
        return ($h{regex_engine} // '') eq 'python-re';
    };
    skip_case('divergence #7: python-re regex engine')
        if any_object($schema, $python_re) || any_object($config, $python_re);
    my @classes = classes($schema);
    my %builtin = map { $_ => 1 } qw(int str float bool bytes list tuple dict set);
    skip_case('divergence #13: model classes by name')
        if grep({ $builtin{$_} } @classes) || has_foreign_model(node_get($case, 'input'), \@classes);
    my $expected = node_get($case, 'expected');
    my $output   = node_get($expected, 'output');
    my $ordered_output = node_has($expected, 'json')
        || (defined $output && !(ref $output eq 'Perldantic::Wire::Ordered' && tag_of([@$output]) eq '$set'));
    my $multi_set = any_object(node_get($case, 'input'), sub ($p) { tag_of($p) eq '$set' && @{$p->[1]} > 1 });
    skip_case('divergence #12: set iteration order') if $multi_set && $ordered_output;
}

sub needs_host_callbacks ($schema) {
    my $feature;
    any_object($schema, sub ($p) {
        my %h = @$p;
        $feature = 'post_init' if defined $h{post_init};
        $feature = 'custom_init' if $h{custom_init};
        return defined $feature;
    });
    skip_case("host callbacks: $feature") if $feature;
}

# ---- comparable values -------------------------------------------------------------------

# What a value looks like after a trip through Perldantic::Wire::decode.
sub plain ($value) {
    my $class = blessed $value // '';
    return {map { $_ } map { $_ % 2 ? plain($value->[$_]) : wire_key($value->[$_]) } 0 .. $#$value}
        if $class eq 'Perldantic::Wire::Ordered';
    return [map { plain($_) } @$value] if $class eq 'Perldantic::Wire::Tuple';
    # Sets have no order: compared as multisets (see eq_value).
    return bless [map { plain($_) } @$value], 'SetCmp' if $class eq 'Perldantic::Wire::Set';
    return $$value if $class eq 'Perldantic::Wire::Bytes';
    return "$value" if $class eq 'Math::BigInt';
    if ($class eq 'Perldantic::Wire::Model') {
        return Perldantic::Wire::Model->new(
            class      => $value->class,
            fields     => plain($value->fields),
            fields_set => bless([map { plain($_) } @{$value->fields_set}], 'SetCmp'),
            extra      => plain($value->extra),
        );
    }
    return [map { plain($_) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => plain($value->{$_}) } keys %$value} if ref $value eq 'HASH';
    return $value;
}

sub wire_key ($key) {
    return $key // '' if !ref $key;
    return "$key" if blessed $key && $key->isa('Math::BigInt');
    # Wire::decode keys a non-scalar dict key by its wire JSON (sorted keys, as Cpanel re-encodes).
    return Cpanel::JSON::XS->new->canonical->allow_nonref->encode(
        Cpanel::JSON::XS->new->decode(Perldantic::Wire::encode($key)));
}

sub describe ($value) {
    return Cpanel::JSON::XS->new->canonical->allow_nonref->allow_blessed->convert_blessed(0)
        ->allow_unknown->encode(plain($value));
}

# ---- running a case ----------------------------------------------------------------------

sub options ($raw) {
    my %options;
    my @pairs = pairs_of($raw);
    while (my ($key, $value) = splice @pairs, 0, 2) {
        $options{$key} = $value;
    }
    return \%options;
}

sub ellipsis_to_true ($node) {
    return [map { ellipsis_to_true($_) } @$node] if ref $node eq 'ARRAY';
    return $node if ref $node ne 'Perldantic::Wire::Ordered';
    my @pairs = @$node;
    return !!1 if @pairs == 2 && $pairs[0] eq '$object' && $pairs[1] eq 'ellipsis';
    return Perldantic::Wire::ordered(map { $_ % 2 ? ellipsis_to_true($pairs[$_]) : $pairs[$_] } 0 .. $#pairs);
}

sub serializer_options ($raw) {
    my $options = options($raw);
    for my $key (keys %$options) {
        my $value = $options->{$key};
        if ($key eq 'include' || $key eq 'exclude') {
            # `...` in a filter means the same as `True`.
            $options->{$key} = interpret(ellipsis_to_true($value));
        }
        elsif (($key eq 'round_trip' || $key eq 'exclude_computed_fields') && !$value) {
            delete $options->{$key};
        }
        elsif (($key eq 'polymorphic_serialization' || $key eq 'context') && !defined $value) {
            delete $options->{$key};
        }
        else {
            $options->{$key} = interpret($value);
        }
    }
    return $options;
}

# Build a validator or serializer, skipping what the core does not support yet.
sub compile ($class, $schema, $config) {
    my $compiled = eval { $class->new($schema, $config) };
    return $compiled if $compiled;
    my $e = $@;
    skip_case('not supported: ' . ((blessed $e ? $e->message : $e) =~ s/\n.*//sr))
        if blessed $e && $e->message =~ /Unknown (?:serialization )?schema type|is not supported yet/;
    die "building failed: $e";
}

# Run a call; unknown keyword options are skipped, as the Rust runner does.
sub run_call ($code) {
    my @warnings;
    local $SIG{__WARN__} = sub ($w) { push @warnings, $w =~ s/ at \S+ line \d+\.?\n\z//r };
    my $result = eval { {ok => $code->()} } // {error => $@};
    if (blessed $result->{error} && $result->{error}->isa('Perldantic::UsageError')
        && $result->{error}->message =~ /unexpected keyword argument '([^']+)'/)
    {
        skip_case("option $1");
    }
    $result->{warnings} = \@warnings;
    return $result;
}

sub check_exception ($result, $want, $with_message) {
    my $e = $result->{error};
    return 'expected an exception, got ' . describe($result->{ok}) if !blessed $e;
    my ($type, $message) = (node_get($want, 'type'), node_get($want, 'message'));
    return "expected $type, got " . ($e->type // ref $e) . ': ' . $e->message if ($e->type // '') ne $type;
    return "expected message '$message', got '" . $e->message . "'" if $with_message && $e->message ne $message;
    return undef;
}

sub run_validator_case ($case) {
    needs_host_callbacks(node_get($case, 'schema'));
    my $schema    = interpret(node_get($case, 'schema'), in_schema => 1);
    my $config    = interpret(node_get($case, 'config'), in_schema => 1);
    my $options   = {map { interpret($_) } %{options(node_get($case, 'options'))}};
    # The Rust runner skips `extra` values the option parser rejects; so does this one.
    skip_case("option extra=$options->{extra}")
        if defined $options->{extra} && $options->{extra} !~ /\A(?:allow|ignore|forbid)\z/;
    my $input     = interpret(node_get($case, 'input'));
    my $expected  = node_get($case, 'expected');
    my $validator = compile('Perldantic::FFI::Validator', $schema, $config);

    my $mode   = node_get($case, 'mode');
    my $result = $mode eq 'json'
        ? do {
            skip_case('non-string JSON input')
                if !defined $input || builtin::is_bool($input)
                || (ref $input && !(blessed $input && $input->isa('Perldantic::Wire::Bytes')));
            my $json = ref $input ? $$input : $input;
            run_call(sub { $validator->validate_json($json, $options) });
        }
        : run_call(sub { $validator->validate($input, $options) });

    if (node_has($expected, 'output')) {
        return 'expected output, got ' . ($result->{error} // '') if exists $result->{error};
        my ($want, $got) = (plain(interpret(node_get($expected, 'output'))), plain($result->{ok}));
        return undef if eq_value($want, $got);
        return 'expected ' . describe($want) . ', got ' . describe($got);
    }
    if (node_has($expected, 'errors')) {
        my $e = $result->{error};
        return 'expected a ValidationError, got ' . (defined $e ? "$e" : describe($result->{ok}))
            if !blessed $e || !$e->isa('Perldantic::ValidationError');
        my $want = plain(interpret(node_get($expected, 'errors')));
        my $got  = [map { my %d = %$_; delete $d{url}; plain(\%d) } @{$e->errors}];
        return 'expected errors ' . describe($want) . ', got ' . describe($got) if !eq_value($want, $got);
        my $title = node_get($expected, 'title');
        return "expected title $title, got " . $e->title if $e->title ne $title;
        return undef;
    }
    return check_exception($result, node_get($expected, 'exception'), 0);
}

sub run_serializer_case ($case) {
    my $schema     = interpret(node_get($case, 'schema'), in_schema => 1);
    my $config     = interpret(node_get($case, 'config'), in_schema => 1);
    my $options    = serializer_options(node_get($case, 'options'));
    my $input      = interpret(node_get($case, 'input'));
    my $expected   = node_get($case, 'expected');
    my $serializer = compile('Perldantic::FFI::Serializer', $schema, $config);
    my $to_json    = node_get($case, 'mode') eq 'to_json';
    my $result = run_call(sub { $to_json ? $serializer->to_json($input, $options) : $serializer->to_python($input, $options) });

    my $want_warnings = interpret(node_get($expected, 'warnings') // []);
    my $check_warnings = sub {
        return undef if eq_value($want_warnings, $result->{warnings});
        return 'expected warnings ' . describe($want_warnings) . ', got ' . describe($result->{warnings});
    };
    if (node_has($expected, 'output')) {
        return 'expected output, got ' . $result->{error} if exists $result->{error};
        my ($want, $got) = (plain(interpret(node_get($expected, 'output'))), plain($result->{ok}));
        return 'expected ' . describe($want) . ', got ' . describe($got) if !eq_value($want, $got);
        return $check_warnings->();
    }
    if (node_has($expected, 'json')) {
        return 'expected JSON, got ' . $result->{error} if exists $result->{error};
        my $want = Encode::encode('UTF-8', node_get($expected, 'json'));
        return "expected JSON $want, got $result->{ok}" if $result->{ok} ne $want;
        return $check_warnings->();
    }
    return check_exception($result, node_get($expected, 'exception'), 1);
}

# Exact comparison of plain values: same shape, same scalars (numbers compared as numbers).
sub eq_value ($a, $b) {
    if (ref $a eq 'SetCmp' || ref $b eq 'SetCmp') {
        return 0 if ref $a !~ /\A(?:SetCmp|ARRAY)\z/ || ref $b !~ /\A(?:SetCmp|ARRAY)\z/ || @$a != @$b;
        my @b = @$b;
        ITEM: for my $item (@$a) {
            for my $i (0 .. $#b) {
                if (eq_value($item, $b[$i])) { splice @b, $i, 1; next ITEM }
            }
            return 0;
        }
        return 1;
    }
    return 0 if ref $a ne ref $b;
    if (ref $a eq 'ARRAY') {
        return 0 if @$a != @$b;
        for (0 .. $#$a) { return 0 if !eq_value($a->[$_], $b->[$_]) }
        return 1;
    }
    if (ref $a eq 'HASH' || ref $a eq 'Perldantic::Wire::Model') {
        return 0 if join("\0", sort keys %$a) ne join("\0", sort keys %$b);
        for (keys %$a) { return 0 if !eq_value($a->{$_}, $b->{$_}) }
        return 1;
    }
    return !defined $b if !defined $a;
    return 0 if !defined $b;
    no warnings 'numeric';
    my $numeric = sub ($x) { !builtin::is_bool($x) && B::svref_2object(\$x)->FLAGS & (B::SVf_IOK | B::SVf_NOK) };
    if ($numeric->($a) && $numeric->($b)) {
        return 1 if $a != $a && $b != $b;    # NaN
        return $a == $b;
    }
    return "$a" eq "$b";
}

sub run_case ($case) {
    divergence($case);
    # The recorder wraps SchemaSerializer, which some pydantic checks reject; that outcome says
    # nothing about pydantic itself.
    my $exception = node_get(node_get($case, 'expected'), 'exception');
    skip_case('recorder artifact') if $exception && (node_get($exception, 'message') // '') =~ /'Recording\w+'/;
    my $mode = node_get($case, 'mode');
    return $mode =~ /\Ato_/ ? run_serializer_case($case) : run_validator_case($case);
}

# ---- the replay --------------------------------------------------------------------------

subtest 'the case reader keeps order and tags' => sub {
    my $node = interpret(parse_json('{"b": {"$tuple": [1, 2.5]}, "a": {"$bytes": "aGk="}, "$c": null}'));
    is ref $node, 'Perldantic::Wire::Ordered';
    is Perldantic::Wire::encode($node), '{"$dict":[["b",{"$tuple":[1,2.5]}],["a",{"$bytes":"aGk="}],["$c",null]]}';
    is interpret(parse_json('{"$class": "M"}'), in_schema => 1), 'M';
    my $skip = dies { interpret(parse_json('{"$function": "f"}')) };
    is $skip->reason, 'value $function';
    is parse_json(qq("caf\\u00e9 \\ud83d\\ude00")), "caf\x{e9} \x{1f600}";
};

my @files;
File::Find::find(sub { push @files, $File::Find::name if /\.json\z/ }, $root);
@files = sort @files;
ok scalar @files, 'case files exist';

my ($total, $passed, %skipped, @failures) = (0, 0);
for my $file (@files) {
    open my $fh, '<:encoding(UTF-8)', $file or die "$file: $!";
    my $cases = parse_json(do { local $/; <$fh> });
    for my $case (@$cases) {
        $total++;
        my $failure = eval { run_case($case) };
        if (my $e = $@) {
            if (blessed $e && $e->isa('Perldantic::Test::CaseJSON::Skip')) {
                $skipped{$e->reason}++;
                next;
            }
            $failure = "died: $e";
        }
        if (defined $failure) { push @failures, node_get($case, 'id') . ": $failure" }
        else                  { $passed++ }
    }
}

my $skipped = 0;
$skipped += $_ for values %skipped;
note sprintf 'conformance: %d cases, %d active, %d passed, %d failed, %d skipped',
    $total, $total - $skipped, $passed, scalar @failures, $skipped;
note sprintf '  skipped %5d: %s', $skipped{$_}, $_ for sort { $skipped{$b} <=> $skipped{$a} || $a cmp $b } keys %skipped;
ok $passed > 0, 'cases are active';
is scalar @failures, 0, 'every active case matches pydantic' or diag join "\n", @failures[0 .. ($#failures < 30 ? $#failures : 29)];

done_testing;
