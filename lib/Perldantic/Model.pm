package Perldantic::Model;

use v5.36;

our $VERSION = '0.01';

use B ();
use Hash::Util::FieldHash ();
use Scalar::Util qw(blessed);
use Sub::Util ();

use Perldantic::Error;
use Perldantic::FFI;
use Perldantic::Role ();
use Perldantic::Temporal ();
use Perldantic::Types ();
use Role::Tiny ();
use Perldantic::Wire;

# Declarations per model class: {fields => [spec, ...], config => {...}, parent => class,
# field_validators => [{fields, mode, code}, ...], model_validators => [{mode, code}, ...],
# field_serializers => [{fields, mode, when_used, return_type, code}, ...],
# model_serializer => {mode, code}, computed_fields => [{name, type, alias}, ...]}.
our %META;
# Compiled validators and serializers per class, and the classes each one was built from (the
# models its schema reaches and their parents). A declaration in a class drops exactly the
# compiled objects built from it.
my (%VALIDATOR, %SERIALIZER, %DEPENDS_ON);
# Per-object state that is not a field: {fields_set => {name => 1}, extra => {...}}.
Hash::Util::FieldHash::fieldhash(my %STATE);

my %HAS_OPTIONS = map { $_ => 1 } qw(
    is isa required default builder lazy predicate clearer init_arg trigger documentation alias
    validate_default
);
my %UNSUPPORTED = map { $_ => 1 } qw(coerce handles weak_ref reader writer moosify);

# pydantic ConfigDict names and the core config key each one becomes.
my %CONFIG_KEY = (
    title                => 'title',
    strict               => 'strict',
    extra                => 'extra_fields_behavior',
    str_strip_whitespace => 'str_strip_whitespace',
    str_to_lower         => 'str_to_lower',
    str_to_upper         => 'str_to_upper',
    str_min_length       => 'str_min_length',
    str_max_length       => 'str_max_length',
    validate_by_name     => 'validate_by_name',
    validate_by_alias    => 'validate_by_alias',
    serialize_by_alias   => 'serialize_by_alias',
    revalidate_instances => 'revalidate_instances',
    json_schema_extra    => 'json_schema_extra',
    url_preserve_empty_path => 'url_preserve_empty_path',
    validate_default     => 'validate_default',
);
# Settings of the Perl layer only; the core never sees them.
my %PERL_SETTING = map { $_ => 1 } qw(temporal_class);

my %CONFIG_FLAG = map { $_ => 1 }
    qw(strict str_strip_whitespace str_to_lower str_to_upper validate_by_name validate_by_alias serialize_by_alias
    url_preserve_empty_path validate_default);

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub _meta ($class) {
    $META{$class} //= {
        fields => [], config => {}, field_validators => [], model_validators => [], field_serializers => [],
        computed_fields => [],
    };
}

# Bumped by every declaration, so that other caches (TypeAdapter) know to rebuild.
our $GENERATION = 0;

sub _changed ($class) {
    for my $user (keys %DEPENDS_ON) {
        next if !$DEPENDS_ON{$user}{$class};
        delete $VALIDATOR{$user};
        delete $SERIALIZER{$user};
        delete $DEPENDS_ON{$user};
    }
    $GENERATION++;
}

# A class and its Perldantic ancestors.
sub _lineage ($class) {
    my @lineage;
    for (my $c = $class; defined $c; $c = _meta($c)->{parent}) { push @lineage, $c }
    return @lineage;
}

sub _is_model ($class) { !ref $class && defined $class && exists $META{$class} }

# ---- declarations -------------------------------------------------------------------------

sub _declare_has ($class, $names, %options) {
    for my $name (ref $names eq 'ARRAY' ? @$names : $names) {
        my $spec = _field_spec($class, $name, %options);
        my $fields = _meta($class)->{fields};
        @$fields = ((grep { $_->{name} ne $name } @$fields), $spec);
        _install_accessors($class, $spec);
    }
    _changed($class);
}

sub _field_spec ($class, $name, %options) {
    my %spec = (name => $name, class => $class);
    my $is = delete $options{is} // 'bare';
    _usage("has $name: 'is' must be ro, rw, rwp, lazy or bare, got '$is'")
        if $is !~ /\A(?:ro|rw|rwp|lazy|bare)\z/;
    if ($is eq 'lazy') {
        $is = 'ro';
        $options{lazy} = 1;
        $options{builder} //= 1 if !exists $options{default};
    }
    $spec{is} = $is;

    my $isa = Perldantic::Types::_as_type(delete $options{isa} // Perldantic::Types::Any());
    _usage("has $name: isa must be a Perldantic type or a Perldantic model class")
        if !blessed $isa || !$isa->isa('Perldantic::Type');

    my %constraints;
    for my $key (sort keys %options) {
        next if $HAS_OPTIONS{$key};
        _usage("has $name: option '$key' is not supported yet") if $UNSUPPORTED{$key};
        _usage("has $name: unknown option '$key'") if !$Perldantic::Type::ANY_CONSTRAINT{$key};
        $constraints{$key} = delete $options{$key};
    }
    $isa = eval { $isa->with(%constraints) } // do {
        my $e = $@;
        _usage("has $name: " . (blessed $e ? $e->message : $e));
    };
    $spec{type} = $isa;

    $spec{required} = !!delete $options{required};
    if (exists $options{default}) {
        my $default = delete $options{default};
        if (ref $default eq 'CODE') {
            $spec{default_code} = $default;
        }
        elsif (ref $default) {
            _usage("has $name: a reference default must be wrapped in a code reference");
        }
        else {
            $spec{default} = [$default];
        }
    }
    if (my $builder = delete $options{builder}) {
        $spec{builder} = $builder eq '1' ? "_build_$name" : $builder;
    }
    $spec{lazy} = !!delete $options{lazy};
    _usage("has $name: lazy needs a default or a builder")
        if $spec{lazy} && !$spec{default_code} && !$spec{builder} && !$spec{default};
    _usage("has $name: a required field cannot have a default")
        if $spec{required} && ($spec{default} || $spec{default_code} || $spec{builder});
    if (exists $options{validate_default}) {
        $spec{validate_default} = !!delete $options{validate_default};
        _usage("has $name: validate_default needs a plain default (code defaults and builders are not validated)")
            if $spec{validate_default} && ($spec{default_code} || $spec{builder});
    }

    if (exists $options{init_arg}) {
        my $init_arg = delete $options{init_arg};
        _usage("has $name: init_arg => undef is not supported yet") if !defined $init_arg;
        $spec{init_arg} = $init_arg;
    }
    $spec{alias} = delete $options{alias};
    for my $kind (qw(predicate clearer)) {
        my $method = delete $options{$kind} // next;
        my $prefix = $kind eq 'predicate' ? 'has' : 'clear';
        $spec{$kind} = $method eq '1' ? ($name =~ /\A_/ ? "_${prefix}$name" : "${prefix}_$name") : $method;
    }
    $spec{trigger} = delete $options{trigger};
    _usage("has $name: trigger must be a code reference")
        if defined $spec{trigger} && ref $spec{trigger} ne 'CODE';
    $spec{documentation} = delete $options{documentation};
    return \%spec;
}

sub _install ($class, $method, $code) {
    no strict 'refs';
    no warnings 'redefine';
    *{"${class}::$method"} = Sub::Util::set_subname("${class}::$method", $code);
}

sub _install_accessors ($class, $spec) {
    my $name = $spec->{name};
    if ($spec->{is} ne 'bare') {
        my $rw = $spec->{is} eq 'rw';
        _install($class, $name, sub ($self, @value) {
            if (@value) {
                _usage("$name is a read-only accessor of " . ref $self) if !$rw;
                _set($self, $spec, $value[0]);
            }
            _fill_lazy($self, $spec) if $spec->{lazy} && !exists $self->{$name};
            return $self->{$name};
        });
    }
    _install($class, "_set_$name", sub ($self, $value) { _set($self, $spec, $value) })
        if $spec->{is} eq 'rwp';
    _install($class, $spec->{predicate}, sub ($self) { exists $self->{$name} }) if $spec->{predicate};
    _install($class, $spec->{clearer}, sub ($self) {
        delete $STATE{$self}{fields_set}{$name};
        delete $self->{$name};
    }) if $spec->{clearer};
}

sub _set ($self, $spec, $value) {
    $self->{$spec->{name}} = $value;
    $STATE{$self}{fields_set}{$spec->{name}} = 1;
    $spec->{trigger}->($self, $value) if $spec->{trigger};
}

sub _fill_lazy ($self, $spec) {
    my $method = $spec->{builder};
    $self->{$spec->{name}} = $spec->{default_code} ? $spec->{default_code}->($self)
        : $spec->{default} ? $spec->{default}[0]
        : $self->$method;
}

sub _declare_extends ($class, $parent) {
    if (!_is_model($parent)) {
        (my $file = "$parent.pm") =~ s{::}{/}g;
        eval { require $file };
    }
    _usage("extends: $parent is not a Perldantic model") if !_is_model($parent);
    no strict 'refs';
    @{"${class}::ISA"} = ($parent);
    _meta($class)->{parent} = $parent;
    _changed($class);
}

sub _declare_with ($class, @roles) {
    Perldantic::Role::_check_roles(@roles);
    eval { Role::Tiny->apply_roles_to_package($class, @roles); 1 } or do {
        my $e = $@;
        _usage('with: ' . (blessed $e ? "$e" : $e =~ s/ at \S+ line \d+\.?\n\z//r));
    };
    _declare_has($class, @$_) for Perldantic::Role::_fields(@roles);
}

my %FIELD_MODE = map { $_ => 1 } qw(before after wrap plain);
my %MODEL_MODE = map { $_ => 1 } qw(before after wrap);

# `field_validator $name | [@names] => (%options) => sub {...}`
sub _declare_field_validator ($class, $fields, @args) {
    my $code = pop @args;
    _usage('field_validator: the last argument must be a code reference') if ref $code ne 'CODE';
    _usage('field_validator: options must be key => value pairs') if @args % 2;
    my %options = @args;
    my $mode = delete $options{mode} // 'after';
    _usage("field_validator: mode must be before, after, wrap or plain, got '$mode'") if !$FIELD_MODE{$mode};
    _usage("field_validator: unknown option '$_'") for sort keys %options;
    my @fields = ref $fields eq 'ARRAY' ? @$fields : ($fields);
    _usage('field_validator: name at least one field') if !@fields || grep { !defined || ref } @fields;
    push @{_meta($class)->{field_validators}}, {fields => \@fields, mode => $mode, code => $code};
    _changed($class);
}

# `model_validator mode => $mode, sub {...}`
sub _declare_model_validator ($class, @args) {
    my $code = pop @args;
    _usage('model_validator: the last argument must be a code reference') if ref $code ne 'CODE';
    _usage('model_validator: options must be key => value pairs') if @args % 2;
    my %options = @args;
    my $mode = delete $options{mode};
    _usage('model_validator: mode must be before, after or wrap') if !defined $mode || !$MODEL_MODE{$mode};
    _usage("model_validator: unknown option '$_'") for sort keys %options;
    push @{_meta($class)->{model_validators}}, {mode => $mode, code => $code};
    _changed($class);
}

my %SERIALIZER_MODE = map { $_ => 1 } qw(plain wrap);
# when_used in Perl's words, and the core's.
my %WHEN_USED = (always => 'always', 'unless-undef' => 'unless-none', json => 'json',
    'json-unless-undef' => 'json-unless-none');

# A Perldantic type from an `isa`-like option (a class name stands for InstanceOf[]).
sub _type_option ($what, $isa) {
    $isa = Perldantic::Types::_as_type($isa);
    _usage("$what: isa must be a Perldantic type or a Perldantic model class")
        if !blessed $isa || !$isa->isa('Perldantic::Type');
    return $isa;
}

sub _serializer_options ($what, %options) {
    my $mode = delete $options{mode} // 'plain';
    _usage("$what: mode must be plain or wrap, got '$mode'") if !$SERIALIZER_MODE{$mode};
    my $when_used = delete $options{when_used} // 'always';
    _usage("$what: when_used must be always, unless-undef, json or json-unless-undef, got '$when_used'")
        if !$WHEN_USED{$when_used};
    $when_used = $WHEN_USED{$when_used};
    my $return_type = delete $options{return_type};
    $return_type = _type_option("$what return_type", $return_type) if defined $return_type;
    _usage("$what: unknown option '$_'") for sort keys %options;
    return (mode => $mode, when_used => $when_used, return_type => $return_type);
}

# `field_serializer $name | [@names] => (%options) => sub {...}`
sub _declare_field_serializer ($class, $fields, @args) {
    my $code = pop @args;
    _usage('field_serializer: the last argument must be a code reference') if ref $code ne 'CODE';
    _usage('field_serializer: options must be key => value pairs') if @args % 2;
    my %options = _serializer_options('field_serializer', @args);
    my @fields = ref $fields eq 'ARRAY' ? @$fields : ($fields);
    _usage('field_serializer: name at least one field') if !@fields || grep { !defined || ref } @fields;
    push @{_meta($class)->{field_serializers}}, {%options, fields => \@fields, code => $code};
    _changed($class);
}

# `model_serializer (%options), sub {...}`
sub _declare_model_serializer ($class, @args) {
    my $code = pop @args;
    _usage('model_serializer: the last argument must be a code reference') if ref $code ne 'CODE';
    _usage('model_serializer: options must be key => value pairs') if @args % 2;
    _meta($class)->{model_serializer} = {_serializer_options('model_serializer', @args), code => $code};
    _changed($class);
}

# `computed_field $name => (isa => $type, alias => $alias) [=> sub {...}]`: without a sub, the
# class's method of that name computes it.
sub _declare_computed_field ($class, $name, @args) {
    my $code = @args % 2 ? pop @args : undef;
    _usage("computed_field $name: the last argument must be a code reference")
        if defined $code && ref $code ne 'CODE';
    my %options = @args;
    my $type = _type_option("computed_field $name", delete $options{isa} // Perldantic::Types::Any());
    my $alias = delete $options{alias};
    _usage("computed_field $name: unknown option '$_'") for sort keys %options;
    _install($class, $name, $code) if $code;
    my $fields = _meta($class)->{computed_fields};
    @$fields = ((grep { $_->{name} ne $name } @$fields), {name => $name, type => $type, alias => $alias});
    _changed($class);
}

sub _declare_config ($class, %settings) {
    for my $key (sort keys %settings) {
        _usage("model_config: unknown setting '$key'") if !$CONFIG_KEY{$key} && !$PERL_SETTING{$key};
        _usage("model_config: temporal_class must be Perldantic, DateTime or Time::Moment, got '$settings{$key}'")
            if $key eq 'temporal_class' && !$Perldantic::Temporal::CLASSES{$settings{$key} // ''};
        _usage("model_config: extra must be allow, ignore or forbid, got '$settings{$key}'")
            if $key eq 'extra' && ($settings{$key} // '') !~ /\A(?:allow|ignore|forbid)\z/;
        _meta($class)->{config}{$key} = $settings{$key};
    }
    _changed($class);
}

# ---- schema -------------------------------------------------------------------------------

# All fields of a class, the parent's first; a redeclared field keeps its place.
sub _fields ($class) {
    my $meta = _meta($class);
    my @fields = $meta->{parent} ? _fields($meta->{parent}) : ();
    for my $spec (@{$meta->{fields}}) {
        my ($at) = grep { $fields[$_]{name} eq $spec->{name} } 0 .. $#fields;
        if (defined $at) { $fields[$at] = $spec } else { push @fields, $spec }
    }
    return @fields;
}

sub _config ($class) {
    my $meta = _meta($class);
    return {($meta->{parent} ? %{_config($meta->{parent})} : ()), %{$meta->{config}}};
}

sub _core_config ($class) {
    return _core_config_from(_config($class), 'model_config');
}

# pydantic config settings as core config, rejecting unknown names.
sub _core_config_from ($config, $what) {
    my %core;
    for my $key (sort keys %$config) {
        next if $PERL_SETTING{$key};
        _usage("$what: unknown setting '$key'") if !$CONFIG_KEY{$key};
        my $value = $config->{$key};
        $core{$CONFIG_KEY{$key}} = $CONFIG_FLAG{$key} ? ($value ? !!1 : !!0) : $value;
    }
    return \%core;
}

# Replace `is-instance` of model classes by references to their definitions.
# The core ref of a model: pydantic's `module.Qualname` shape, so JSON Schema `$defs` are named
# after the last package component (`Shop::Item` is `Item`) unless names collide.
sub _ref ($class) { $class =~ s/::/./gr }

sub _link ($schema, $visit) {
    my $ref = ref $schema;
    # union labels (tuples) and ordered dicts (Dict[] fields, tagged-union choices) hold schemas too
    return Perldantic::Wire::tuple(map { _link($_, $visit) } @$schema) if $ref eq 'Perldantic::Wire::Tuple';
    return Perldantic::Wire::ordered(map { _link($_, $visit) } @$schema) if $ref eq 'Perldantic::Wire::Ordered';
    if ($ref eq 'HASH') {
        if (($schema->{type} // '') eq 'is-instance' && _is_model($schema->{cls})) {
            $visit->($schema->{cls});
            return {type => 'definition-ref', schema_ref => _ref($schema->{cls})};
        }
        return {map { $_ => _link($schema->{$_}, $visit) } keys %$schema};
    }
    return [map { _link($_, $visit) } @$schema] if ref $schema eq 'ARRAY';
    return $schema;
}

# How many arguments a sub takes, or undef when it takes any number (no signature, or a slurpy
# one): pydantic passes `info` only to functions that take it.
sub _max_args ($code) {
    my $cv = B::svref_2object($code);
    for (my $op = $cv->START; $op && $$op; $op = $op->next) {
        next if $op->name =~ /\A(?:nextstate|dbstate|null|enter)\z/;
        if ($op->name eq 'argcheck') {
            my ($params, undef, $slurpy) = $op->aux_list($cv);
            return $slurpy ? undef : $params;
        }
        last;
    }
    return undef;
}

sub _takes ($code, $count) {
    my $max = _max_args($code);
    return !defined $max || $max >= $count;
}

# Validated data from the core, as the functions of a model see it: models become objects.
# Validation tracks objects, so the objects a function returns come back unchanged.
sub _live ($value) { _inflate($value) }

sub _live_handler ($handler) {
    return sub (@args) { _live($handler->(@args)) };
}

# A function schema calling `$code` through `$call`, named after it.
sub _function_schema ($mode, $code, $call, $schema) {
    Sub::Util::set_subname(Sub::Util::subname($code), $call);
    return {
        type     => "function-$mode",
        function => {type => 'with-info', function => $call},
        # a plain validator takes any input, as pydantic's JSON Schema says
        ($mode eq 'plain' ? (json_schema_input_schema => {type => 'any'}) : (schema => $schema)),
    };
}

# The validators of a class and its ancestors that apply to a field, oldest first.
sub _field_validators ($class) {
    return map { @{_meta($_)->{field_validators}} } reverse _lineage($class);
}

sub _field_validator_schema ($class, $validator, $schema) {
    my ($mode, $code) = @$validator{qw(mode code)};
    my $info = _takes($code, $mode eq 'wrap' ? 4 : 3);
    my $call = $mode eq 'wrap'
        ? sub ($value, $handler, $i) { $code->($class, _live($value), _live_handler($handler), $info ? $i : ()) }
        : sub ($value, $i) { $code->($class, _live($value), $info ? $i : ()) };
    return _function_schema($mode, $code, $call, $schema);
}

sub _model_validator_schema ($class, $validator, $schema) {
    my ($mode, $code) = @$validator{qw(mode code)};
    my $call;
    if ($mode eq 'after') {
        my $info = _takes($code, 2);
        $call = sub ($model, $i) { $code->(_live($model), $info ? $i : ()) };
    } elsif ($mode eq 'before') {
        my $info = _takes($code, 3);
        $call = sub ($data, $i) { $code->($class, _live($data), $info ? $i : ()) };
    } else {
        my $info = _takes($code, 4);
        $call = sub ($data, $handler, $i) { $code->($class, _live($data), _live_handler($handler), $info ? $i : ()) };
    }
    return _function_schema($mode, $code, $call, $schema);
}

# The `serialization` of a schema with a serializer function: `$args` builds the Perl
# arguments from the core's.
sub _serializer_function ($spec, $call, $visit, %extra) {
    my ($mode, $code) = @$spec{qw(mode code)};
    Sub::Util::set_subname(Sub::Util::subname($code), $call);
    return {
        type      => "function-$mode",
        function  => $call,
        info_arg  => !!1,
        when_used => $spec->{when_used},
        ($spec->{return_type} ? (return_schema => _link($spec->{return_type}->core_schema, $visit)) : ()),
        %extra,
    };
}

sub _field_serialization ($serializer, $visit) {
    my $code = $serializer->{code};
    my $call;
    if ($serializer->{mode} eq 'wrap') {
        my $info = _takes($code, 4);
        $call = sub ($model, $value, $handler, $i) { $code->(_live($model), _live($value), $handler, $info ? $i : ()) };
    } else {
        my $info = _takes($code, 3);
        $call = sub ($model, $value, $i) { $code->(_live($model), _live($value), $info ? $i : ()) };
    }
    return _serializer_function($serializer, $call, $visit, is_field_serializer => !!1);
}

sub _model_serialization ($serializer, $visit) {
    my $code = $serializer->{code};
    my $call;
    if ($serializer->{mode} eq 'wrap') {
        my $info = _takes($code, 3);
        $call = sub ($model, $handler, $i) { $code->(_live($model), $handler, $info ? $i : ()) };
    } else {
        my $info = _takes($code, 2);
        $call = sub ($model, $i) { $code->(_live($model), $info ? $i : ()) };
    }
    return _serializer_function($serializer, $call, $visit);
}

# The computed fields of a class and its ancestors (a redeclared one keeps its place).
sub _computed_fields ($class) {
    my @fields;
    for my $field (map { @{_meta($_)->{computed_fields}} } reverse _lineage($class)) {
        my ($at) = grep { $fields[$_]{name} eq $field->{name} } 0 .. $#fields;
        if (defined $at) { $fields[$at] = $field } else { push @fields, $field }
    }
    return @fields;
}

sub _computed_field_schema ($class, $field, $visit) {
    my $name = $field->{name};
    _usage("computed_field: $class has no method '$name'") if !$class->can($name);
    my $getter = Sub::Util::set_subname("${class}::$name", sub ($model, $property) { _live($model)->$property });
    return {
        type          => 'computed-field',
        property_name => $name,
        return_schema => _link($field->{type}->core_schema, $visit),
        function      => $getter,
        (defined $field->{alias} ? (alias => $field->{alias}) : ()),
        # as pydantic marks them
        metadata => {pydantic_js_updates => {readOnly => !!1}},
    };
}

sub _computed_fields_schema ($class, $visit) {
    my @fields = _computed_fields($class) or return ();
    return (computed_fields => [map { _computed_field_schema($class, $_, $visit) } @fields]);
}

sub _field_schema ($spec, $visit, $for_json_schema) {
    my $schema = _link($spec->{type}->core_schema, $visit);
    for my $validator (@{$spec->{validators} // []}) {
        $schema = _field_validator_schema($spec->{owner}, $validator, $schema);
    }
    $schema = {%$schema, serialization => _field_serialization($spec->{serializer}, $visit)} if $spec->{serializer};
    if ($spec->{default}) {
        $schema = {type => 'default', schema => $schema, default => $spec->{default}[0],
            (defined $spec->{validate_default} ? (validate_default => $spec->{validate_default}) : ())};
    }
    elsif (!$spec->{required}) {
        # Validation needs a default to leave the field out; Perl fills or removes it afterwards.
        # A JSON Schema shows no default, as pydantic does for default factories.
        # The placeholder is never validated, whatever model_config says.
        $schema = {type => 'default', schema => $schema,
            ($for_json_schema ? () : (default => undef, validate_default => !!0))};
    }
    my $alias = $spec->{init_arg} // $spec->{alias};
    return {
        type   => 'model-field',
        schema => $schema,
        (defined $alias ? (validation_alias => $alias) : ()),
        (defined $spec->{alias} ? (serialization_alias => $spec->{alias}) : ()),
    };
}

sub _model_schema ($class, $visit, $for_json_schema) {
    my $config = _core_config($class);
    my @fields = _fields($class);
    my %validators;
    for my $validator (_field_validators($class)) {
        for my $name (@{$validator->{fields}}) {
            _usage("field_validator: $class has no field '$name'") if !grep { $_->{name} eq $name } @fields;
            push @{$validators{$name}}, $validator;
        }
    }
    # one serializer per field: the latest declared, a subclass's over its parent's
    my %serializer;
    for my $serializer (map { @{_meta($_)->{field_serializers}} } reverse _lineage($class)) {
        for my $name (@{$serializer->{fields}}) {
            _usage("field_serializer: $class has no field '$name'") if !grep { $_->{name} eq $name } @fields;
            $serializer{$name} = $serializer;
        }
    }
    my ($model_serializer) = grep {defined} map { _meta($_)->{model_serializer} } _lineage($class);
    my $schema = {
        type   => 'model',
        cls    => $class,
        schema => {
            type       => 'model-fields',
            model_name => $class,
            fields     => Perldantic::Wire::ordered(map {
                my $spec = {%$_, owner => $class, validators => $validators{$_->{name}}, serializer => $serializer{$_->{name}}};
                ($_->{name} => _field_schema($spec, $visit, $for_json_schema));
            } @fields),
            _computed_fields_schema($class, $visit),
        },
        (%$config ? (config => $config) : ()),
        ($model_serializer ? (serialization => _model_serialization($model_serializer, $visit)) : ()),
    };
    for my $validator (map { @{_meta($_)->{model_validators}} } reverse _lineage($class)) {
        $schema = _model_validator_schema($class, $validator, $schema);
    }
    # the reference names the outermost schema, which the model's validators wrap
    return {%$schema, ref => _ref($class)};
}

# The core schema of a class: its model and every model it refers to, as definitions.
sub core_schema ($class, %options) {
    return _linked_schema({type => 'is-instance', cls => $class}, $options{for_json_schema}, $options{depends_on});
}

# A core schema in which model classes (`is-instance` of a model) refer to definitions of their
# model schemas, collected with every model they reach in turn.
sub _linked_schema ($schema, $for_json_schema = 0, $depends_on = {}) {
    my (%seen, @definitions);
    # __SUB__ rather than a closure over $visit, which would be a reference cycle.
    my $visit = sub ($model) {
        return if $seen{$model}++;
        $depends_on->{$_} = 1 for _lineage($model);
        push @definitions, _model_schema($model, __SUB__, !!$for_json_schema);
    };
    my $linked = _link($schema, $visit);
    return $linked if !@definitions;
    return {type => 'definitions', schema => $linked, definitions => \@definitions};
}

sub _compiled ($cache, $compiler, $class) {
    return $cache->{$class} //= do {
        my %depends_on;
        # errors are titled with the model, whatever functions wrap its schema (as pydantic)
        my $title = _config($class)->{title} // $class;
        my $compiled = $compiler->new($class->core_schema(depends_on => \%depends_on), {title => $title});
        $DEPENDS_ON{$class} = {%{$DEPENDS_ON{$class} // {}}, %depends_on};
        $compiled;
    };
}

sub _validator ($class)  { _compiled(\%VALIDATOR,  'Perldantic::FFI::Validator',  $class) }
sub _serializer ($class) { _compiled(\%SERIALIZER, 'Perldantic::FFI::Serializer', $class) }

# ---- instances ----------------------------------------------------------------------------

sub BUILDARGS ($class, @args) {
    return {@args} if @args % 2 == 0;
    return {%{$args[0]}} if @args == 1 && ref $args[0] eq 'HASH';
    _usage("$class->new expects a hash or a hash reference");
}

# Model objects given as input to a validation, by token. While a validation runs, objects are
# sent with a token among their set field names (which no message shows); the core returns
# objects it does not revalidate unchanged, and _inflate hands back the original object for
# them, as pydantic keeps instances.
our (%INPUT_OBJECTS, $TRACK_OBJECTS);
my $TOKEN_PREFIX = "\0perldantic object ";

sub _input_object ($model) {
    for my $name (@{$model->fields_set}) {
        next if $name !~ /\A\Q$TOKEN_PREFIX\E(\d+)\z/;
        my $object = $INPUT_OBJECTS{$1};
        return $object && ref $object eq $model->class ? $object : undef;
    }
    return undef;
}

# Serialize with objects tracked: serializer functions and computed fields get the very objects
# being dumped.
our $DUMPING;

sub _dump_tracked ($code) {
    local $TRACK_OBJECTS = 1;
    local $DUMPING = 1;
    local %INPUT_OBJECTS;
    return $code->();
}

sub _validate_tracked ($code, $args = undef) {
    local $TRACK_OBJECTS = 1;
    local %INPUT_OBJECTS;
    my $result = eval { $code->() };
    if (!defined $result && (my $e = $@)) {
        _untrack_error($e) if blessed $e && $e->isa('Perldantic::ValidationError');
        die $e;
    }
    return _inflate($result, $args);
}

# In a validation error, inputs that were model objects become those objects again.
sub _untrack_error ($e) {
    $_->{input} = _untrack($_->{input}) for @{$e->{errors}};
}

sub _untrack ($value) {
    if (blessed $value && $value->isa('Perldantic::Wire::Model')) {
        return _input_object($value) // $value;
    }
    return [map { _untrack($_) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => _untrack($value->{$_}) } keys %$value} if ref $value eq 'HASH';
    return $value;
}

sub new ($class, @args) {
    _usage('new is a class method') if ref $class;
    my $args = $class->BUILDARGS(@args);
    return _validate_tracked(sub { $class->_validator->validate($args) }, $args);
}

sub does ($self, $role) { Role::Tiny::does_role($self, $role) }

# What inflating objects of a class needs, worked out once per class and declaration
# generation: its fields, their temporal class, its BUILD methods (parents first).
my %PLAN;

sub _plan ($class) {
    my $plan = $PLAN{$class};
    return $plan if $plan && $plan->{generation} == $GENERATION;
    my @builds = grep {defined} map {
        no strict 'refs';
        *{"${_}::BUILD"}{CODE};
    } reverse @{mro::get_linear_isa($class)};
    my @fields = _fields($class);
    my $config = _config($class);
    return $PLAN{$class} = {
        generation => $GENERATION,
        fields     => \@fields,
        names      => [map { $_->{name} } @fields],
        temporal   => $config->{temporal_class} // 'Perldantic',
        revalidate => ($config->{revalidate_instances} // 'never') eq 'always',
        builds     => \@builds,
    };
}

# Turn validated data from the core into objects, building nested models first.
sub _inflate ($value, $args = undef) {
    my $ref = ref $value or return $value;
    if ($ref eq 'Perldantic::Wire::Model') {
        my $class  = $value->{class};
        my $fields = $value->{fields};
        my $fields_set = $value->{fields_set};
        my %set;
        for my $name (@$fields_set) {
            if (index($name, $TOKEN_PREFIX) == 0) {
                my $object = _input_object($value);
                return $object if $object;
                next;
            }
            $set{$name} = 1;
        }
        my $plan = _plan($class);
        my $temporal = $plan->{temporal};
        my $self = bless {
            map {
                my $field = ref $fields->{$_} ? _inflate($fields->{$_}) : $fields->{$_};
                ($_ => $temporal eq 'Perldantic' ? $field : Perldantic::Temporal::_convert_deep($field, $temporal));
            } keys %$fields
        }, $class;
        $STATE{$self} = {fields_set => \%set, extra => $value->{extra}};
        for my $spec (@{$plan->{fields}}) {
            my $name = $spec->{name};
            if ($set{$name}) {
                $spec->{trigger}->($self, $self->{$name}) if $spec->{trigger};
                next;
            }
            if ($spec->{lazy} || (!$spec->{default} && !$spec->{default_code} && !$spec->{builder})) {
                delete $self->{$name};
            }
            elsif (!$spec->{default}) {
                _fill_lazy($self, $spec);
            }
        }
        if (@{$plan->{builds}}) {
            $args //= {%$fields};
            $self->$_($args) for @{$plan->{builds}};
        }
        return $self;
    }
    return [map { ref $_ ? _inflate($_) : $_ } @$value] if $ref eq 'ARRAY';
    return {map { $_ => (ref $value->{$_} ? _inflate($value->{$_}) : $value->{$_}) } keys %$value} if $ref eq 'HASH';
    return $value;
}

sub _build ($self, $args) {
    $self->$_($args) for @{_plan(ref $self)->{builds}};
}

# ---- pydantic methods ---------------------------------------------------------------------

sub _class_method ($name, $class) {
    _usage("$name is a class method") if ref $class;
}

sub _object_method ($name, $self) {
    _usage("$name is an object method") if !ref $self;
}

sub _options ($name, @options) {
    _usage("$name takes options as key => value pairs") if @options % 2;
    return {@options};
}

# Dump options in Perl's words (exclude_undef, mode => 'perl'), as the core takes them.
sub _dump_options ($name, @options) {
    my $options = _options($name, @options);
    _usage("$name: unknown option 'exclude_none'; undefined values are left out with exclude_undef")
        if exists $options->{exclude_none};
    $options->{exclude_none} = delete $options->{exclude_undef} if exists $options->{exclude_undef};
    if (defined $options->{mode} && !ref $options->{mode}) {
        _usage("$name: mode must be perl, json or a name of your own, got 'python'") if $options->{mode} eq 'python';
        $options->{mode} = 'python' if $options->{mode} eq 'perl';
    }
    return $options;
}

sub model_validate ($class, $data, @options) {
    _class_method('model_validate', $class);
    my $options = _options('model_validate', @options);
    return _validate_tracked(sub { $class->_validator->validate($data, $options) });
}

sub model_validate_json ($class, $json, @options) {
    _class_method('model_validate_json', $class);
    my $options = _options('model_validate_json', @options);
    return _validate_tracked(sub { $class->_validator->validate_json($json, $options) });
}

sub model_dump ($self, @options) {
    _object_method('model_dump', $self);
    my $options = _dump_options('model_dump', @options);
    return _dump_tracked(sub { ref($self)->_serializer->to_perl($self, $options) });
}

sub model_dump_json ($self, @options) {
    _object_method('model_dump_json', $self);
    my $options = _dump_options('model_dump_json', @options);
    return _dump_tracked(sub { ref($self)->_serializer->to_json($self, $options) });
}

sub model_json_schema ($class, @options) {
    _class_method('model_json_schema', $class);
    return Perldantic::FFI::json_schema($class->core_schema(for_json_schema => 1), undef, _options('model_json_schema', @options));
}

sub model_fields_set ($self) {
    _object_method('model_fields_set', $self);
    return sort keys %{$STATE{$self}{fields_set}};
}

sub model_extra ($self) {
    _object_method('model_extra', $self);
    return $STATE{$self}{extra};
}

sub _deep_copy ($value) {
    return $value->model_copy(deep => 1) if blessed $value && $value->isa('Perldantic::Model');
    return $value if blessed $value;
    return [map { _deep_copy($_) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => _deep_copy($value->{$_}) } keys %$value} if ref $value eq 'HASH';
    return $value;
}

sub model_copy ($self, %options) {
    _object_method('model_copy', $self);
    for my $key (sort keys %options) {
        _usage("model_copy: unknown option '$key'") if $key ne 'update' && $key ne 'deep';
    }
    my $copy = $options{deep} ? _deep_copy({%$self}) : {%$self};
    bless $copy, ref $self;
    my $state = $STATE{$self};
    my $extra = $state->{extra};
    $STATE{$copy} = {
        fields_set => {%{$state->{fields_set}}},
        extra      => defined $extra ? ($options{deep} ? _deep_copy($extra) : {%$extra}) : undef,
    };
    my %field = map { $_->{name} => 1 } _fields(ref $self);
    my $update = $options{update} // {};
    for my $key (sort keys %$update) {
        if ($field{$key}) {
            $copy->{$key} = $update->{$key};
            $STATE{$copy}{fields_set}{$key} = 1;
        }
        elsif (defined $STATE{$copy}{extra}) {
            $STATE{$copy}{extra}{$key} = $update->{$key};
        }
        else {
            _usage("model_copy: " . ref($self) . " has no field '$key'");
        }
    }
    return $copy;
}

# The wire JSON of the object, written directly (what encoding _perldantic_wire's result
# gives, without building it).
sub _wire_json ($self) {
    my $plan = _plan(ref $self);
    my $fields = Perldantic::Wire::_object_any([map { exists $self->{$_} ? ($_ => $self->{$_}) : () } @{$plan->{names}}]);
    my @set = sort keys %{$STATE{$self}{fields_set} // {}};
    if ($TRACK_OBJECTS && ($DUMPING || !$plan->{revalidate})) {
        my $token = Scalar::Util::refaddr($self);
        $INPUT_OBJECTS{$token} = $self;
        push @set, "$TOKEN_PREFIX$token";
    }
    return '{"$model":{"class":' . Perldantic::Wire::_string(ref $self)
        . ',"extra":' . Perldantic::Wire::_emit_any($STATE{$self}{extra})
        . ',"fields":' . $fields
        . ',"fields_set":' . Perldantic::Wire::_emit_any(\@set) . '}}';
}

sub _perldantic_wire ($self) {
    my $plan = _plan(ref $self);
    my @names = @{$plan->{names}};
    my @token;
    # Objects the core revalidates keep only their real field names.
    if ($TRACK_OBJECTS && ($DUMPING || !$plan->{revalidate})) {
        my $token = Scalar::Util::refaddr($self);
        $INPUT_OBJECTS{$token} = $self;
        @token = ("$TOKEN_PREFIX$token");
    }
    return Perldantic::Wire::Model->new(
        class      => ref $self,
        fields     => Perldantic::Wire::ordered(map { exists $self->{$_} ? ($_ => $self->{$_}) : () } @names),
        fields_set => [(sort keys %{$STATE{$self}{fields_set} // {}}), @token],
        extra      => $STATE{$self}{extra},
    );
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Model - base class of Perldantic models

=head1 SYNOPSIS

    package Point;
    use Perldantic;               # makes Point a Perldantic::Model

    has x => (is => 'ro', isa => Int, required => 1);
    has y => (is => 'ro', isa => Int, default => 0);

    package main;
    my $p = Point->new(x => '3');                 # validated: $p->x == 3
    my $q = Point->model_validate({x => 1, y => 2});
    my $r = Point->model_validate_json('{"x": 5}');

    my $data = $p->model_dump;                     # {x => 3, y => 0}
    my $json = $p->model_dump_json;                # {"x":3,"y":0}
    my $set  = $p->model_fields_set;               # ['x']
    my $moved = $p->model_copy(update => {y => 9});
    my $schema = Point->model_json_schema;

=head1 DESCRIPTION

Every class that says C<use Perldantic> inherits from this one. The declarations of the class
(see L<Perldantic>) become a pydantic C<model> core schema. The validator and the serializer of
a class are compiled on first use and cached. A later declaration drops only the compiled objects
built from the class it changes: its own, its subclasses' and those of models that contain it.

Objects are blessed hashes of their fields, like Moo objects. Nested models are objects too, and
a field that was neither given nor defaulted is absent from the hash.

=head1 METHODS

=head2 new(%args) / new(\%args)

Validates the arguments and returns the object. Invalid input raises
C<Perldantic::ValidationError>. Arguments that are not a hash raise C<Perldantic::UsageError>.
C<BUILDARGS> and C<BUILD> work as in Moo: C<BUILD> methods run parent first, with the arguments
hash.

=head2 BUILDARGS(@args)

Turns the arguments of C<new> into a hash reference, as in Moo; override it to take other
arguments. A list must be key-value pairs, or a single hash reference.

=head2 core_schema(%options)

Class method: the core schema of the model, with every model it refers to as a definition.
With C<< for_json_schema => 1 >>, fields that have no plain default show none.

=head2 model_validate($data, %options), model_validate_json($json, %options)

Class methods: validate Perl data or JSON text (bytes or characters) into an object. Options
are pydantic's: C<strict>, C<extra>, C<from_attributes>, C<context>, C<by_alias>, C<by_name>.

=head2 model_dump(%options), model_dump_json(%options)

The object as Perl data, or as UTF-8 encoded JSON. Options are pydantic's, in Perl's words:
C<mode> (C<perl>, the default, C<json>, or a name of your own for serializer functions to see),
C<include>, C<exclude> (hashes of names with true values, nested as in pydantic, or arrays of
names), C<by_alias>, C<exclude_unset>, C<exclude_defaults>, C<exclude_undef> (pydantic's
C<exclude_none>), C<warnings>, and for JSON C<indent> and C<ensure_ascii>. Fields absent from the object are left out.

=head2 model_json_schema(%options)

Class method: the JSON Schema, as Perl data. Options: C<mode>, C<by_alias>, C<ref_template>,
C<union_format>. Nested models are C<$defs> named after the last component of their package.

=head2 model_copy(update => \%fields, deep => $bool)

A copy of the object. C<update> values are not validated (as in pydantic) and count as set;
C<deep> copies nested data and models.

=head2 model_fields_set

The names of the fields that were given or written, sorted.

=head2 model_extra

The extra fields (with C<< extra => 'allow' >>), or C<undef>.

=head2 does($role)

Whether the class consumes the role (see L<Perldantic::Role>).

=head1 MODEL OBJECTS AS INPUT

A model object given as input (to C<new>, C<model_validate> or a
L<Perldantic::TypeAdapter>) is kept as it is, as in pydantic, unless its class sets
C<< revalidate_instances => 'always' >>; then a validated copy is made.


=head1 SEE ALSO

L<Perldantic> (declaring models), L<Perldantic::TypeAdapter>, L<Perldantic::Error>

=cut
