package Perldantic::Model;

use v5.36;

our $VERSION = '0.01';

use Hash::Util::FieldHash ();
use Scalar::Util qw(blessed);
use Sub::Util ();

use Perldantic::Error;
use Perldantic::FFI;
use Perldantic::Types ();
use Perldantic::Wire;

# Declarations per model class: {fields => [spec, ...], config => {...}, parent => class}.
our %META;
# Compiled validators per class, dropped whenever any declaration changes.
my %VALIDATOR;
# Per-object state that is not a field: {fields_set => {name => 1}, extra => {...}}.
Hash::Util::FieldHash::fieldhash(my %STATE);

my %HAS_OPTIONS = map { $_ => 1 } qw(
    is isa required default builder lazy predicate clearer init_arg trigger documentation alias
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
);
my %CONFIG_FLAG = map { $_ => 1 }
    qw(strict str_strip_whitespace str_to_lower str_to_upper validate_by_name validate_by_alias serialize_by_alias);

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub _meta ($class) { $META{$class} //= {fields => [], config => {}} }

sub _changed () { %VALIDATOR = () }

sub _is_model ($class) { !ref $class && defined $class && exists $META{$class} }

# ---- declarations -------------------------------------------------------------------------

sub _declare_has ($class, $names, %options) {
    for my $name (ref $names eq 'ARRAY' ? @$names : $names) {
        my $spec = _field_spec($class, $name, %options);
        my $fields = _meta($class)->{fields};
        @$fields = ((grep { $_->{name} ne $name } @$fields), $spec);
        _install_accessors($class, $spec);
    }
    _changed();
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

    my $isa = delete $options{isa} // Perldantic::Types::Any();
    if (!ref $isa) {
        $isa = Perldantic::Types::InstanceOf([$isa]);
    }
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
    _changed();
}

sub _declare_config ($class, %settings) {
    for my $key (sort keys %settings) {
        _usage("model_config: unknown setting '$key'") if !$CONFIG_KEY{$key};
        _usage("model_config: extra must be allow, ignore or forbid, got '$settings{$key}'")
            if $key eq 'extra' && ($settings{$key} // '') !~ /\A(?:allow|ignore|forbid)\z/;
        _meta($class)->{config}{$key} = $settings{$key};
    }
    _changed();
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
    my $config = _config($class);
    my %core;
    for my $key (keys %$config) {
        my $value = $config->{$key};
        $core{$CONFIG_KEY{$key}} = $CONFIG_FLAG{$key} ? ($value ? !!1 : !!0) : $value;
    }
    return \%core;
}

# Replace `is-instance` of model classes by references to their definitions.
sub _link ($schema, $visit) {
    if (ref $schema eq 'HASH') {
        if (($schema->{type} // '') eq 'is-instance' && _is_model($schema->{cls})) {
            $visit->($schema->{cls});
            return {type => 'definition-ref', schema_ref => $schema->{cls}};
        }
        return {map { $_ => _link($schema->{$_}, $visit) } keys %$schema};
    }
    return [map { _link($_, $visit) } @$schema] if ref $schema eq 'ARRAY';
    return $schema;
}

sub _field_schema ($spec, $visit) {
    my $schema = _link($spec->{type}->core_schema, $visit);
    if (!$spec->{required}) {
        $schema = {type => 'default', schema => $schema, default => $spec->{default} ? $spec->{default}[0] : undef};
    }
    my $alias = $spec->{init_arg} // $spec->{alias};
    return {
        type   => 'model-field',
        schema => $schema,
        (defined $alias ? (validation_alias => $alias) : ()),
        (defined $spec->{alias} ? (serialization_alias => $spec->{alias}) : ()),
    };
}

sub _model_schema ($class, $visit) {
    my $config = _core_config($class);
    return {
        type   => 'model',
        cls    => $class,
        ref    => $class,
        schema => {
            type       => 'model-fields',
            model_name => $class,
            fields     => Perldantic::Wire::ordered(map { ($_->{name} => _field_schema($_, $visit)) } _fields($class)),
        },
        (%$config ? (config => $config) : ()),
    };
}

# The core schema of a class: its model and every model it refers to, as definitions.
sub core_schema ($class) {
    my (%seen, @definitions);
    my $visit;
    $visit = sub ($model) {
        return if $seen{$model}++;
        push @definitions, _model_schema($model, $visit);
    };
    $visit->($class);
    return {
        type        => 'definitions',
        schema      => {type => 'definition-ref', schema_ref => $class},
        definitions => \@definitions,
    };
}

sub _validator ($class) {
    return $VALIDATOR{$class} //= Perldantic::FFI::Validator->new($class->core_schema);
}

# ---- instances ----------------------------------------------------------------------------

sub BUILDARGS ($class, @args) {
    return {@args} if @args % 2 == 0;
    return {%{$args[0]}} if @args == 1 && ref $args[0] eq 'HASH';
    _usage("$class->new expects a hash or a hash reference");
}

sub new ($class, @args) {
    _usage('new is a class method') if ref $class;
    my $args = $class->BUILDARGS(@args);
    return _inflate($class->_validator->validate($args), $args);
}

# Turn validated data from the core into objects, building nested models first.
sub _inflate ($value, $args = undef) {
    if (blessed $value && $value->isa('Perldantic::Wire::Model')) {
        my $class  = $value->class;
        my $fields = $value->fields;
        my $self   = bless {map { $_ => _inflate($fields->{$_}) } keys %$fields}, $class;
        my %set    = map { $_ => 1 } @{$value->fields_set};
        $STATE{$self} = {fields_set => \%set, extra => $value->extra};
        for my $spec (_fields($class)) {
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
        _build($self, $args // {%$fields});
        return $self;
    }
    return [map { _inflate($_) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => _inflate($value->{$_}) } keys %$value} if ref $value eq 'HASH' && !blessed $value;
    return $value;
}

sub _build ($self, $args) {
    for my $class (reverse @{mro::get_linear_isa(ref $self)}) {
        my $build = do { no strict 'refs'; *{"${class}::BUILD"}{CODE} } // next;
        $self->$build($args);
    }
}

sub _perldantic_wire ($self) {
    my @names = map { $_->{name} } _fields(ref $self);
    return Perldantic::Wire::Model->new(
        class      => ref $self,
        fields     => {map { exists $self->{$_} ? ($_ => $self->{$_}) : () } @names},
        fields_set => [sort keys %{$STATE{$self}{fields_set} // {}}],
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
    my $p = Point->new(x => '3');  # validated: $p->x == 3

=head1 DESCRIPTION

Every class that says C<use Perldantic> inherits from this one. The declarations of the class
(see L<Perldantic>) become a pydantic C<model> core schema. The validator is compiled on first
use and rebuilt after any later declaration.

Objects are blessed hashes of their fields, like Moo objects. Nested models are objects too, and
a field that was neither given nor defaulted is absent from the hash.

=head1 METHODS

=head2 new(%args) / new(\%args)

Validates the arguments and returns the object. Invalid input raises
C<Perldantic::ValidationError>. Arguments that are not a hash raise C<Perldantic::UsageError>.
C<BUILDARGS> and C<BUILD> work as in Moo: C<BUILD> methods run parent first, with the arguments
hash.

=head2 core_schema

Class method: the core schema of the model, with every model it refers to as a definition.

=head1 LIMITATIONS

A model object passed as input to another model's field is copied, not shared, and the copy
runs C<BUILD> again.

=cut
