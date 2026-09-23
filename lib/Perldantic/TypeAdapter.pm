package Perldantic::TypeAdapter;

use v5.36;

our $VERSION = '0.01';

use Scalar::Util qw(blessed);

use Perldantic::Error;
use Perldantic::FFI;
use Perldantic::Model;
use Perldantic::Types ();

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub new ($class, @args) {
    _usage('TypeAdapter->new takes a type') if !@args;
    my ($type, %options) = @args;
    for my $key (sort keys %options) {
        _usage("TypeAdapter->new: unknown option '$key'") if $key ne 'config';
    }
    my $self = bless {config => $options{config} // {}}, $class;
    if (!ref $type && Perldantic::Model::_is_model($type)) {
        _usage("TypeAdapter: config cannot be used with a model class; use model_config in $type")
            if %{$self->{config}};
        $self->{model} = $type;
        $self->{type}  = Perldantic::Types::InstanceOf([$type]);
    }
    elsif (!ref $type && defined $type && $type =~ /\A\w+(?:::\w+)*\z/) {
        $self->{type} = Perldantic::Types::InstanceOf([$type]);
    }
    elsif (blessed $type && $type->isa('Perldantic::Type')) {
        $self->{type} = $type;
    }
    else {
        _usage('TypeAdapter->new takes a Perldantic type or a model class, got ' . (ref $type || $type // 'undef'));
    }
    # Checked now, so that a bad setting fails where the adapter is made.
    $self->{core_config} = Perldantic::Model::_core_config_from($self->{config}, 'TypeAdapter: config');
    return $self;
}

# The core config for validation and serialization: errors are titled with the Perl type name
# unless the config gives a title.
sub _runtime_config ($self) {
    return undef if $self->{model};
    return {title => $self->{type}->name, %{$self->{core_config}}};
}

sub _compiled ($self, $kind) {
    if (($self->{generation} // -1) != $Perldantic::Model::GENERATION) {
        delete @$self{qw(validator serializer)};
        $self->{generation} = $Perldantic::Model::GENERATION;
    }
    return $self->{$kind} //= do {
        my $schema = Perldantic::Model::_linked_schema($self->{type}->core_schema);
        my $new    = $kind eq 'validator' ? 'Perldantic::FFI::Validator' : 'Perldantic::FFI::Serializer';
        $new->new($schema, $self->_runtime_config);
    };
}

sub _options ($name, @options) {
    _usage("$name takes options as key => value pairs") if @options % 2;
    return {@options};
}

sub validate_python ($self, $data, @options) {
    my $result = $self->_compiled('validator')->validate($data, _options('validate_python', @options));
    return Perldantic::Model::_inflate($result);
}

sub validate_json ($self, $json, @options) {
    my $result = $self->_compiled('validator')->validate_json($json, _options('validate_json', @options));
    return Perldantic::Model::_inflate($result);
}

sub dump_python ($self, $value, @options) {
    return $self->_compiled('serializer')->to_python($value, _options('dump_python', @options));
}

sub dump_json ($self, $value, @options) {
    return $self->_compiled('serializer')->to_json($value, _options('dump_json', @options));
}

sub json_schema ($self, @options) {
    my $schema = Perldantic::Model::_linked_schema($self->{type}->core_schema, 1);
    my $config = $self->{model} ? undef : $self->{core_config};
    return Perldantic::FFI::json_schema($schema, ($config && %$config ? $config : undef), _options('json_schema', @options));
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::TypeAdapter - validate and serialize any Perldantic type

=head1 SYNOPSIS

    use Perldantic::TypeAdapter;
    use Perldantic::Types qw(ArrayRef Int);

    my $ints = Perldantic::TypeAdapter->new(ArrayRef[Int]);
    my $list = $ints->validate_python([1, '2']);      # [1, 2]
    my $json = $ints->dump_json($list);               # '[1,2]'
    my $js   = $ints->json_schema;                    # {type => 'array', items => {type => 'integer'}}

    my $points = Perldantic::TypeAdapter->new(ArrayRef['My::Point']);   # models inside types

=head1 DESCRIPTION

pydantic's C<TypeAdapter>: the model API for a type that is not a model. Models inside the type
(C<InstanceOf['Class']> or a class name) are validated into objects. The adapter compiles its
validator and serializer on first use and rebuilds them after later model declarations.

=head1 METHODS

=head2 new($type, config => \%config)

C<$type> is a L<Perldantic::Types> type or a model class name. C<config> takes the settings of
C<model_config> (see L<Perldantic>), and cannot be combined with a model class. Validation
errors are titled with the type name (C<ArrayRef[Int]>) unless the config sets C<title>.

=head2 validate_python($data, %options), validate_json($json, %options)

Validate Perl data or JSON text; options as for C<model_validate>.

=head2 dump_python($value, %options), dump_json($value, %options)

Serialize to Perl data or UTF-8 encoded JSON; options as for C<model_dump>.

=head2 json_schema(%options)

The JSON Schema, as Perl data; options as for C<model_json_schema>.

Misuse (a missing or wrong type, unknown options or config settings) raises
C<Perldantic::UsageError>.

=cut
