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

    package Ticket;
    use Perldantic;                        # instead of `use Moo;`

    has id     => (is => 'ro', isa => Int, required => 1, gt => 0);
    has title  => (is => 'rw', isa => Str, required => 1, max_length => 200);
    has status => (is => 'ro', isa => Enum[qw(open done)], default => 'open');
    has tags   => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
    has owner  => (is => 'ro', isa => Maybe['Person']);   # another Perldantic model
    model_config extra => 'forbid';

    package main;
    my $t = Ticket->new(id => '42', title => 'bug');       # "42" becomes 42

=head1 DESCRIPTION

Perldantic brings pydantic's data validation to Perl and keeps its philosophy: declared types
are the schema, input is parsed into guaranteed types, and errors are structured data. The
engine is a Python-free port of C<pydantic-core>.

C<use Perldantic> turns the package into a model class (a L<Perldantic::Model>), enables
C<strict> and C<warnings>, and imports C<has>, C<extends>, C<model_config>,
C<field_validator>, C<model_validator> and the types of
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

A L<Perldantic::Types> type, or the name of a Perldantic model class. Defaults to C<Any>.

=item C<required>

The field must be given. Other fields without a default may be left out and are then absent.

=item C<default>, C<builder>, C<lazy>

A plain default value, or a code reference / builder method called with the object. As in
pydantic, defaults are not validated. C<lazy> delays code defaults and builders until the
field is first read.

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
C<revalidate_instances>, C<url_preserve_empty_path> and C<json_schema_extra>; and, for the Perl
layer only, C<temporal_class> (see L<Perldantic::Temporal>).

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

=head1 LICENSE

MIT. Includes software derived from pydantic-core (MIT); see F<NOTICE>.

=cut
