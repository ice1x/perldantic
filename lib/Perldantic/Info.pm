package Perldantic::Info;

use v5.36;

our $VERSION = '0.01';

# What validator functions that take `info` get (pydantic's ValidationInfo).
package Perldantic::ValidationInfo {
    sub new ($class, %args) { bless {%args}, $class }

    sub config ($self)     { $self->{config} }
    sub context ($self)    { $self->{context} }
    sub data ($self)       { $self->{data} }
    sub field_name ($self) { $self->{field_name} }
    sub mode ($self)       { $self->{mode} }
}

# What serializer functions that take `info` get (pydantic's SerializationInfo).
package Perldantic::SerializationInfo {
    # The core's words (mode `python`, exclude_none) in Perl's.
    sub new ($class, %args) {
        $args{mode} = 'perl' if ($args{mode} // '') eq 'python';
        $args{exclude_undef} = delete $args{exclude_none} if exists $args{exclude_none};
        return bless {%args}, $class;
    }

    sub include ($self)                 { $self->{include} }
    sub exclude ($self)                 { $self->{exclude} }
    sub context ($self)                 { $self->{context} }
    sub mode ($self)                    { $self->{mode} }
    sub mode_is_json ($self)            { $self->{mode} eq 'json' }
    sub by_alias ($self)                { $self->{by_alias} }
    sub exclude_unset ($self)           { $self->{exclude_unset} }
    sub exclude_defaults ($self)        { $self->{exclude_defaults} }
    sub exclude_undef ($self)           { $self->{exclude_undef} }
    sub exclude_computed_fields ($self) { $self->{exclude_computed_fields} }
    sub round_trip ($self)              { $self->{round_trip} }
    sub serialize_as_any ($self)        { $self->{serialize_as_any} }
    sub field_name ($self)              { $self->{field_name} }
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Info - the info argument of validator and serializer functions

=head1 SYNOPSIS

    package Signup;
    use Perldantic;

    has password => (is => 'ro', isa => Str, required => 1);
    has confirm  => (is => 'ro', isa => Str, required => 1);

    # A validator that takes three arguments gets a Perldantic::ValidationInfo.
    field_validator confirm => sub ($class, $value, $info) {
        die "passwords differ\n" if $value ne $info->data->{password};
        return $value;
    };

    # So does a serializer that takes three: a Perldantic::SerializationInfo.
    field_serializer password => sub ($self, $value, $info) {
        return $info->mode_is_json ? '***' : $value;
    };

    package main;
    my $signup = Signup->new(password => 'secret', confirm => 'secret');
    say $signup->model_dump_json;      # {"password":"***","confirm":"secret"}

=head1 DESCRIPTION

Validator and serializer functions that ask for it get an C<info> object describing the call, as
pydantic's functions get C<ValidationInfo> or C<SerializationInfo>. A function asks for it by
taking one more argument (see L<Perldantic/field_validator>, L<Perldantic/field_serializer> and
L<Perldantic::FFI/Functions>).

Perldantic makes these objects; they are read-only.

=head1 Perldantic::ValidationInfo

=head2 new(%fields)

Makes the object; Perldantic calls it.

=head2 config

The config of the schema being validated, a hash reference or C<undef>.

=head2 context

The C<context> given to the validation call, or C<undef>.

=head2 data

For field validators: the fields validated so far, a hash reference.

=head2 field_name

For field validators: the name of the field.

=head2 mode

How the input was given: C<perl> (Perl data) or C<json> (JSON text).

=head1 Perldantic::SerializationInfo

=head2 new(%fields)

Makes the object; Perldantic calls it.

=head2 mode

The output mode: C<perl>, C<json> or another name given as C<mode> to C<model_dump> or C<dump>.

=head2 mode_is_json

True when the output is JSON: C<model_dump_json>, C<dump_json> or C<< mode => 'json' >>.

=head2 include, exclude

The C<include> and C<exclude> options of the call.

=head2 context

The C<context> given to the call, or C<undef>.

=head2 by_alias, exclude_unset, exclude_defaults, exclude_undef, exclude_computed_fields, round_trip, serialize_as_any

The flags of the call, as given to C<model_dump> or C<dump>.

=head2 field_name

For field serializers: the name of the field.

=head1 SEE ALSO

L<Perldantic>, L<Perldantic::Model>, L<Perldantic::TypeAdapter>

=cut
