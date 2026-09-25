package Perldantic;

use v5.36;

our $VERSION = '0.01';

use Class::Method::Modifiers ();

use Perldantic::Model;
use Perldantic::Role ();
use Perldantic::Types ();

# `use Perldantic;` turns the calling package into a model class, like `use Moo;`.
sub import ($class, @args) {
    my $target = caller;
    Perldantic::UsageError->throw(message => "use Perldantic takes no arguments, got @args") if @args;
    strict->import;
    warnings->import;
    no strict 'refs';
    push @{"${target}::ISA"}, 'Perldantic::Model' if !$target->isa('Perldantic::Model');
    Perldantic::Model::_meta($target);
    *{"${target}::has"}          = sub ($names, @options) { Perldantic::Model::_declare_has($target, $names, @options) };
    *{"${target}::extends"}      = sub ($parent) { Perldantic::Model::_declare_extends($target, $parent) };
    *{"${target}::model_config"} = sub (%settings) { Perldantic::Model::_declare_config($target, %settings) };
    *{"${target}::with"}         = sub (@roles) { Perldantic::Model::_declare_with($target, @roles) };
    *{"${target}::field_validator"} = sub ($fields, @args) { Perldantic::Model::_declare_field_validator($target, $fields, @args) };
    *{"${target}::model_validator"} = sub (@args) { Perldantic::Model::_declare_model_validator($target, @args) };
    *{"${target}::field_serializer"} = sub ($fields, @args) { Perldantic::Model::_declare_field_serializer($target, $fields, @args) };
    *{"${target}::model_serializer"} = sub (@args) { Perldantic::Model::_declare_model_serializer($target, @args) };
    *{"${target}::computed_field"}   = sub ($name, @args) { Perldantic::Model::_declare_computed_field($target, $name, @args) };
    for my $type (qw(before after around)) {
        *{"${target}::$type"} = sub (@args) { Class::Method::Modifiers::install_modifier($target, $type, @args) };
    }
    *{"${target}::$_"} = \&{"Perldantic::Types::$_"} for @Perldantic::Types::EXPORT_OK;
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic - pydantic for Perl, powered by a Rust core

=head1 SYNOPSIS

    package Person;
    use Perldantic;                        # instead of `use Moo;`

    has name  => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has email => (is => 'ro', isa => Str, pattern => '^[^@]+@[^@]+$');

    package Ticket;
    use Perldantic;

    has id     => (is => 'ro', isa => Int, required => 1, gt => 0);
    has title  => (is => 'rw', isa => Str, required => 1, max_length => 200);
    has status => (is => 'ro', isa => Enum[qw(open done)], default => 'open');
    has tags   => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
    has owner  => (is => 'ro', isa => Maybe['Person']);   # another Perldantic model
    model_config extra => 'forbid';

    field_validator title => sub ($class, $title) { ucfirst $title };

    package main;
    use Scalar::Util qw(blessed);

    my $t = Ticket->new(id => '42', title => 'bug', owner => {name => 'Ann'});
    say $t->id;                            # 42: "42" became a number
    say $t->owner->name;                   # Ann: the hash became a Person
    say $t->model_dump_json;               # {"id":42,"title":"Bug",...,"owner":{"name":"Ann"}}
    my $schema = Ticket->model_json_schema;   # JSON Schema, as Perl data

    my $copy = Ticket->model_validate_json('{"id": 7, "title": "crash"}');

    eval { Ticket->new(id => 0, title => 'x', colour => 'red') };
    if (blessed $@ && $@->isa('Perldantic::ValidationError')) {
        say $@->error_count;               # 2: id is not > 0, colour is not a field
        say $_->{type} for @{$@->errors};  # greater_than, extra_forbidden
    }

=head1 DESCRIPTION

Perldantic brings pydantic's data validation to Perl and keeps its philosophy: declared types
are the schema, input is parsed into guaranteed types, and errors are structured data. The
engine is a Python-free port of C<pydantic-core>.

C<use Perldantic> turns the package into a model class (a L<Perldantic::Model>), enables
C<strict> and C<warnings>, and imports C<has>, C<extends>, C<model_config>,
C<field_validator>, C<model_validator>, C<field_serializer>, C<model_serializer>,
C<computed_field> and the types of
L<Perldantic::Types>, as well as C<with> and the method modifiers C<before>, C<after> and
C<around> (L<Class::Method::Modifiers>).

=head1 DECLARATIONS

=head2 has $name => (%options), has [@names] => (%options)

Declares a field. Moo options:

=over

=item C<is>

C<ro>, C<rw>, C<rwp> (read-only, with a C<_set_$name> writer), C<lazy> (C<ro> plus C<lazy>,
with a C<_build_$name> builder unless there is a default) or C<bare> (no accessor, the default).

=item C<isa>

A L<Perldantic::Types> type, a L<Type::Tiny> constraint, or a class name: a Perldantic model
class, or any other class, whose objects the field then takes (C<InstanceOf[]>). Defaults to
C<Any>.

=item C<required>

The field must be given. Other fields without a default may be left out and are then absent.

=item C<default>, C<builder>, C<lazy>

A plain default value, or a code reference / builder method called with the object. As in
pydantic, defaults are not validated unless C<< validate_default => 1 >> (or the
C<validate_default> model config) asks for it; only plain defaults can be. C<lazy> delays code
defaults and builders until the field is first read.

=item C<predicate>, C<clearer>

Generate C<has_$name> and C<clear_$name> (or the given method names).

=item C<init_arg>

The name the constructor takes the field under.

=item C<trigger>

A code reference called with the object and the value when the field is given or written.

=item C<documentation>

Kept, not used.

=back

Perldantic options: C<alias> (pydantic's alias, for input and output) and the constraints of the
type, such as C<gt>, C<min_length>, C<pattern> or C<strict> (see L<Perldantic::Type>). Any other
option raises C<Perldantic::UsageError>. C<coerce>, C<handles>, C<weak_ref>, C<reader> and
C<writer> are not supported yet.

=head2 extends $parent

Inherits from another Perldantic model: its fields come first and its C<model_config> applies.

=head2 with @roles

Consumes L<Perldantic::Role> roles: their methods and modifiers as in Moo, and their fields as
if declared in the class.

=head2 model_config %settings

pydantic's model config: C<title>, C<strict>, C<extra> (C<allow>, C<ignore> or C<forbid>),
C<str_strip_whitespace>, C<str_to_lower>, C<str_to_upper>, C<str_min_length>,
C<str_max_length>, C<validate_by_name>, C<validate_by_alias>, C<serialize_by_alias>,
C<revalidate_instances>, C<url_preserve_empty_path>, C<validate_default> and C<json_schema_extra>; and, for the Perl
layer only, C<temporal_class> (see L<Perldantic::Temporal>) and C<lazy> (C<new>,
C<model_validate> and C<model_validate_json> return lazy objects, see
L<Perldantic::Model/LAZY OBJECTS>).

=head2 field_validator $name | [@names] => (mode => $mode) => sub {...}

pydantic's C<@field_validator>: a function that validates one or more fields, called as a class
method. C<mode> is:

=over

=item C<after> (the default)

C<< sub ($class, $value, $info) >> gets the value the field's type produced and returns the
field's value.

=item C<before>

The same, with the input as given, before the field's type checks it.

=item C<plain>

The same, instead of the field's type.

=item C<wrap>

C<< sub ($class, $value, $handler, $info) >>: C<< $handler->($value) >> runs the field's type.

=back

C<$info> (a L<Perldantic::ValidationInfo> with C<field_name>, the C<data> validated so far and the
C<context>) is passed only to subs whose signature takes it. A validator reports invalid input
by dying: with a string (a C<value_error> with that message) or with the error classes of
L<Perldantic::Error/Classes>; any other exception object is raised unchanged. Validators of a
parent class apply to its subclasses, in declaration order after the parent's; naming a field
the class does not have is a C<Perldantic::UsageError> when the class is first used.

=head2 model_validator mode => $mode, sub {...}

pydantic's C<@model_validator>, for checks across fields. C<mode> is required:

=over

=item C<after>

C<< sub ($self, $info) >> gets the object once its fields are validated and returns it (it may
change it through its accessors).

=item C<before>

C<< sub ($class, $data, $info) >> gets the input and returns the data to validate.

=item C<wrap>

C<< sub ($class, $data, $handler, $info) >>: C<< $handler->($data) >> validates and returns the
object.

=back

Errors have an empty location.

=head2 field_serializer $name | [@names] => (%options) => sub {...}

pydantic's C<@field_serializer>: how one or more fields are dumped, called as an object method.
Options:

=over

=item C<mode>

C<plain> (the default): C<< sub ($self, $value, $info) >> returns the dumped value. C<wrap>:
C<< sub ($self, $value, $handler, $info) >>, where C<< $handler->($value) >> dumps the value
as the field's type would.

=item C<when_used>

C<always> (the default), C<unless-undef> (not for undef), C<json> (only C<model_dump_json> and
C<< mode => 'json' >>) or C<json-unless-undef>.

=item C<return_type>

The type of what the sub returns, which then serializes it and describes it in the
serialization JSON Schema.

=back

C<$info> is a L<Perldantic::SerializationInfo>, passed to subs that take it. One serializer
applies per field: the latest declared, a subclass's over its parent's.

=head2 model_serializer (%options), sub {...}

pydantic's C<@model_serializer>: how the whole object is dumped. C<plain> (the default):
C<< sub ($self, $info) >> returns the dumped data; C<wrap>: C<< sub ($self, $handler, $info) >>,
where C<< $handler->($self) >> gives the fields as usual. Takes C<when_used> and C<return_type>
like C<field_serializer>.

=head2 computed_field $name => (isa => $type, alias => $alias) => sub {...}

pydantic's C<@computed_field>: a value computed from the object and dumped with the fields. The
sub is installed as the method C<$name>; without a sub, the class's own method C<$name> is
used. Computed fields are not input; they appear after the fields in dumps (subject to
C<include> / C<exclude> and C<by_alias>) and in the serialization JSON Schema, marked
C<readOnly>.

=head1 SEE ALSO

L<Perldantic::Model> (the methods of model objects), L<Perldantic::Types> (the types),
L<Perldantic::Type> (constraints), L<Perldantic::TypeAdapter> (validating any type),
L<Perldantic::Role>, L<Perldantic::Error>, L<Perldantic::Temporal>, L<Perldantic::Uuid>,
L<Perldantic::Url>, L<Perldantic::FFI> (the core schema API).

F<docs/DIVERGENCES.md> lists where Perldantic differs from pydantic, and
L<https://docs.pydantic.dev> documents the behaviour it follows.

=head1 AUTHOR

ice1x

=head1 LICENSE

MIT. Includes software derived from pydantic-core (MIT); see F<NOTICE>.

=cut
