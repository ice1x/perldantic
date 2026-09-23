package Perldantic::Role;

use v5.36;

our $VERSION = '0.01';

use Role::Tiny ();

use Perldantic::Error;
use Perldantic::Types ();

# Field declarations per role, replayed into every class that consumes the role:
# {role => [[$names, @options], ...]}.
our %FIELDS;
# The roles each role consumes itself.
our %WITH;

# `use Perldantic::Role;` makes the calling package a role, like `use Moo::Role;`.
sub import ($class, @args) {
    my $target = caller;
    Perldantic::UsageError->throw(message => "use Perldantic::Role takes no arguments, got @args") if @args;
    eval "package $target; use Role::Tiny; 1" or die $@;    ## no critic (ProhibitStringyEval)
    $FIELDS{$target} //= [];
    no strict 'refs';
    no warnings 'redefine';
    *{"${target}::has"} = sub ($names, @options) { push @{$FIELDS{$target}}, [$names, @options] };
    *{"${target}::with"} = sub (@roles) {
        _check_roles(@roles);
        push @{$WITH{$target}}, @roles;
        Role::Tiny->apply_roles_to_package($target, @roles);
    };
    *{"${target}::$_"} = \&{"Perldantic::Types::$_"} for @Perldantic::Types::EXPORT_OK;
}

sub _check_roles (@roles) {
    for my $role (@roles) {
        if (!Role::Tiny->is_role($role)) {
            (my $file = "$role.pm") =~ s{::}{/}g;
            eval { require $file };
        }
        Perldantic::UsageError->throw(message => "with: $role is not a role") if !Role::Tiny->is_role($role);
    }
}

# Every field declaration of the roles, those of the roles they consume first.
sub _fields (@roles) {
    my (%seen, @fields);
    my $collect = sub ($role) {
        return if $seen{$role}++;
        __SUB__->($_) for @{$WITH{$role} // []};
        push @fields, @{$FIELDS{$role} // []};
    };
    $collect->($_) for @roles;
    return @fields;
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Role - roles for Perldantic models

=head1 SYNOPSIS

    package Named;
    use Perldantic::Role;                  # instead of `use Moo::Role;`

    has name => (is => 'ro', isa => Str, required => 1);
    requires 'kind';
    sub label ($self) { $self->kind . ': ' . $self->name }

    package Doc;
    use Perldantic;
    with 'Named';
    sub kind {'doc'}

=head1 DESCRIPTION

A role built on L<Role::Tiny>: its methods, C<requires> and method modifiers (C<before>,
C<after>, C<around>) behave as in Moo roles. Its C<has> declarations become fields of every
model that consumes it, validated like the model's own. Roles can consume roles with C<with>.

Consuming something that is not a role, or a role whose required methods are missing, raises
C<Perldantic::UsageError>.

=cut
