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

Perldantic::Info - the C<info> argument of validator and serializer functions

=head1 DESCRIPTION

Functions in schemas (see L<Perldantic::FFI/Functions>) that ask for it get an C<info> object
describing the call, as pydantic's functions get C<ValidationInfo> or C<SerializationInfo>.

=head2 Perldantic::ValidationInfo

C<config> (the schema's config), C<context> (given to the validation call), C<data> (the fields
validated so far, for field validators), C<field_name> and C<mode> (C<perl>, C<python> or
C<json>: how the input was given).

=head2 Perldantic::SerializationInfo

C<include>, C<exclude>, C<context>, C<mode> (C<perl>, C<json> or another name given to
C<model_dump> or C<dump>) and C<mode_is_json>, C<by_alias>, C<exclude_unset>, C<exclude_defaults>,
C<exclude_undef>, C<exclude_computed_fields>, C<round_trip>, C<serialize_as_any> and, for field
serializers, C<field_name>.

=cut
