package Perldantic::Call;

use v5.36;

our $VERSION = '0.01';

use Exporter 'import';
use Scalar::Util qw(blessed);
use Sub::Util ();

use Perldantic::Arguments;
use Perldantic::Error;
use Perldantic::FFI;
use Perldantic::Model;
use Perldantic::Types ();
use Perldantic::Wire;

our @EXPORT_OK = qw(validate_call);

my %OPTION = map { $_ => 1 } qw(positional named returns method config);

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub validate_call ($target, %options) {
    my ($code, $name, $install);
    if (ref $target eq 'CODE') {
        $code = $target;
        $name = Sub::Util::subname($code);
    }
    elsif (defined $target && !ref $target) {
        $name = $target =~ /::/ ? $target : caller() . "::$target";
        no strict 'refs';
        _usage("validate_call: $name is not a sub") if !defined &{$name};
        $code = \&{$name};
        $install = 1;
    }
    else {
        _usage('validate_call takes a sub name or a code reference');
    }
    for my $key (sort keys %options) {
        _usage("validate_call: unknown option '$key'") if !$OPTION{$key};
    }

    my $self = _plan($name, \%options);
    my $method = !!$options{method};
    my $wrapper = sub {
        my @invocant = $method ? shift : ();
        # positional arguments alone: the core reads @_ in place
        my ($args, $kwargs) = @{_validate($self, $self->{named} ? _arguments($self, \@_) : \@_)};
        @_ = (@invocant, @$args, %$kwargs);
        goto &$code if !$self->{returns};
        return _validate_return($self, scalar $code->(@_));
    };
    Sub::Util::set_subname($name, $wrapper);
    if ($install) {
        no strict 'refs';
        no warnings 'redefine';
        *{$name} = $wrapper;
    }
    return $wrapper;
}

# The parameters as core `arguments` schema parts, checked now so that mistakes show where the
# sub is declared; the schema itself is built on first call (models may be declared later).
sub _plan ($name, $options) {
    my $self = {name => $name, params => [], positional => 0, generation => -1};
    my $had_default;
    for my $kind (qw(positional named)) {
        my $specs = $options->{$kind} // next;
        _usage("validate_call: $kind takes an array reference") if ref $specs ne 'ARRAY';
        my @specs = @$specs;
        my ($index, %seen) = (0);
        while (@specs) {
            # a slurpy type needs no name
            my $slurpy_next = blessed $specs[0] && $specs[0]->isa('Perldantic::Type') && $specs[0]->is_slurpy;
            my $param_name = $kind eq 'named' && !$slurpy_next ? shift @specs : $index;
            my $what = $kind eq 'named' ? "named argument '" . ($param_name // 'undef') . "'" : "$kind\[$index]";
            _usage("validate_call: named takes name => type pairs") if $kind eq 'named' && (!defined $param_name || ref $param_name);
            _usage("validate_call: $what is declared twice") if $seen{$param_name}++;
            my $type = Perldantic::Types::_as_type(shift @specs);
            _usage("validate_call: $what takes a type") if !blessed $type || !$type->isa('Perldantic::Type');
            my $param_options = ref $specs[0] eq 'HASH' ? shift @specs : {};
            for my $key (sort keys %$param_options) {
                _usage("validate_call: $what: unknown option '$key'") if $key ne 'default';
            }
            $index++;
            if ($type->is_slurpy) {
                _usage("validate_call: slurpy is only allowed last") if @specs;
                my $slot = $type->{slurpy} eq 'ArrayRef' ? 'var_args' : 'var_kwargs';
                _usage('validate_call: slurpy ArrayRef belongs to positional, slurpy HashRef to named')
                    if ($slot eq 'var_args') != ($kind eq 'positional');
                $self->{$slot} = $type;
                next;
            }
            my $param = {name => "$param_name", kind => $kind, type => $type};
            if (exists $param_options->{default}) {
                my $default = $param_options->{default};
                _usage("validate_call: $what: a default must be a plain value")
                    if ref $default && !(blessed $default && $default->isa('Perldantic::Enum'));
                $param->{default} = [$default];
                $had_default = 1 if $kind eq 'positional';
            }
            elsif ($had_default && $kind eq 'positional') {
                _usage("validate_call: $what has no default but follows one that does");
            }
            $self->{positional}++ if $kind eq 'positional';
            push @{$self->{params}}, $param;
        }
    }
    _usage('validate_call: slurpy positional arguments cannot be combined with named ones')
        if $self->{var_args} && ($options->{named} && @{$options->{named}});
    $self->{named} = grep { $_->{kind} eq 'named' } @{$self->{params}};
    $self->{named} ||= !!$self->{var_kwargs};
    $self->{returns} = Perldantic::Types::_as_type($options->{returns}) if defined $options->{returns};
    _usage('validate_call: returns takes a type')
        if exists $options->{returns} && !(blessed $self->{returns} && $self->{returns}->isa('Perldantic::Type'));
    $self->{config} = {title => $name,
        %{Perldantic::Model::_core_config_from($options->{config} // {}, 'validate_call: config')}};
    return $self;
}

# What the core validates for a call with named arguments: a hash of them (read in place), or
# Perldantic::Arguments with positional ones too.
sub _arguments ($self, $given) {
    my @named = @$given;
    my @positional = splice @named, 0, $self->{positional};
    my $kwargs;
    if (@named == 1 && ref $named[0] eq 'HASH') {
        $kwargs = $named[0];
    }
    else {
        _usage("$self->{name} takes named arguments as name => value pairs") if @named % 2;
        $kwargs = {@named};
    }
    return $self->{positional} ? Perldantic::Arguments->new(args => \@positional, kwargs => $kwargs) : $kwargs;
}

sub _schema ($self) {
    my @params = map {
        my $schema = $_->{type}->core_schema;
        $schema = {type => 'default', schema => $schema, default => $_->{default}[0]} if $_->{default};
        {name => $_->{name}, mode => $_->{kind} eq 'named' ? 'keyword_only' : 'positional_only', schema => $schema};
    } @{$self->{params}};
    return {
        type             => 'arguments',
        arguments_schema => \@params,
        ($self->{var_args}   ? (var_args_schema   => $self->{var_args}->core_schema)   : ()),
        ($self->{var_kwargs} ? (var_kwargs_schema => $self->{var_kwargs}->core_schema) : ()),
    };
}

# A validator, compiled on first use and again after later model declarations, and whether its
# results may hold models (which then need tracking and building into objects).
sub _compiled ($self, $kind, $schema) {
    if (($self->{generation} // -1) != $Perldantic::Model::GENERATION) {
        delete @$self{qw(arguments return)};
        $self->{generation} = $Perldantic::Model::GENERATION;
    }
    return @{$self->{$kind} //= do {
        my $linked = Perldantic::Model::_linked_schema($schema->());
        [Perldantic::FFI::Validator->new($linked, $self->{config}), $linked->{type} eq 'definitions'];
    }};
}

sub _validated ($validator, $models, $input) {
    return $validator->validate($input) if !$models;
    return Perldantic::Model::_validate_tracked(sub { $validator->validate($input) });
}

sub _validate ($self, $arguments) {
    my $compiled = $self->{generation} == $Perldantic::Model::GENERATION && $self->{arguments};
    return _validated($compiled ? @$compiled : _compiled($self, arguments => sub { _schema($self) }), $arguments);
}

# The result, validated as the field `return` of a dict: errors are located under `return`.
sub _validate_return ($self, $value) {
    my @compiled = _compiled($self, return => sub {
        {type => 'typed-dict', fields => Perldantic::Wire::ordered(
            return => {type => 'typed-dict-field', schema => $self->{returns}->core_schema})};
    });
    return _validated(@compiled, {return => $value})->{return};
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Call - validate the arguments of subs

=head1 SYNOPSIS

    use v5.36;
    use Perldantic::Call qw(validate_call);
    use Perldantic::Types qw(Int Num ArrayRef slurpy);

    validate_call add => (positional => [Int, Int, {default => 1}], returns => Int);
    sub add ($x, $y) { $x + $y }

    add(2, '3');                     # 5
    add(2);                          # 3
    eval { add('x') };               # $@ is a Perldantic::ValidationError

    validate_call area => (named => [width => Int, height => Int, {default => 1}]);
    sub area (%size) { $size{width} * $size{height} }

    area(width => 10, height => 2);  # 20
    area({width => 10});             # 10

    my $sum = validate_call(sub (@n) { my $s = 0; $s += $_ for @n; $s },
        positional => [slurpy ArrayRef[Num]]);
    $sum->(1, '2.5');                # 3.5

=head1 DESCRIPTION

pydantic's C<validate_call>, with the options of L<Type::Params>' C<signature_for>: the
arguments are validated (and converted, as fields are) by the core C<arguments> schema before
the sub runs, and the sub gets the validated values. Invalid arguments raise a
C<Perldantic::ValidationError> titled with the sub's name, located by argument position or
name, as in pydantic.

=head1 FUNCTIONS

=head2 validate_call($name, %options), validate_call($code, %options)

With a sub name (of the calling package unless qualified), the sub is replaced by the
validating one; with a code reference, the validating sub is returned. Options:

=over

=item C<< positional => [$type, ...] >>

Positional arguments, in order. A type may be followed by C<< {default => $value} >> (a plain
value or an enum member), used when the argument is left out; later positional arguments then
need a default too. A last C<slurpy ArrayRef[T]> takes the remaining arguments, each validated
as C<T>.

=item C<< named => [name => $type, ...] >>

Named arguments, given as C<< name => value >> pairs after the positional ones (all positional
arguments must then be given) or as one hash reference. Defaults as for positional arguments. A
last C<slurpy HashRef[T]> takes other names, each validated as C<T>; without it, unknown names
are errors. The sub gets its named arguments as pairs.

=item C<< returns => $type >>

The sub is called in scalar context and its result validated, errors being located under
C<return>. Without it, the sub is called in the caller's context and its result returned as it
is; the validating sub does not show on the call stack.

=item C<< method => 1 >>

The first argument is the invocant, passed on unvalidated.

=item C<< config => \%config >>

Settings of C<model_config> for validation (C<strict>, ...).

=back

Types are L<Perldantic::Types> types, Type::Tiny constraints or class names, as for C<isa>;
models are built from hash references and model objects given are kept. The schema is built
on first call, so models may be declared after the sub. Mistakes in the options raise
C<Perldantic::UsageError>.

=head1 SEE ALSO

L<Perldantic>, L<Perldantic::Types>, L<Perldantic::Arguments>, L<Type::Params>

=cut
