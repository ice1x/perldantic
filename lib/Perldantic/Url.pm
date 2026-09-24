package Perldantic::Url;

use v5.36;

our $VERSION = '0.01';

use Scalar::Util ();

use Perldantic::Error;

# A URL as pydantic's `Url` has it: the text the core normalised (it stringifies to it), with
# accessors the core computes on first use. Perldantic::MultiHostUrl below shares the code.
use overload
    '""'     => sub ($self, @) { $self->{text} },
    'cmp'    => sub ($a, $b, $swap) { my $c = $a->_compare($b); $swap ? -$c : $c },
    'eq'     => sub ($a, $b, @) { $a->_equal($b) },
    'ne'     => sub ($a, $b, @) { !$a->_equal($b) },
    bool     => sub { 1 },
    fallback => 1;

sub _core_type ($class) { 'url' }

sub _usage ($class, $message) {
    Perldantic::UsageError->throw(message => (ref($class) || $class) . "->$message");
}

my %VALIDATORS;

# Parse and normalise URL text as `Url(text, preserve_empty_path=...)` does; invalid text
# raises Perldantic::ValidationError.
sub new ($class, $text, %options) {
    my @unknown = grep { $_ ne 'preserve_empty_path' } sort keys %options;
    $class->_usage("new: unknown option '$unknown[0]'") if @unknown;
    my $preserve = $options{preserve_empty_path} ? 1 : 0;
    require Perldantic::FFI;
    my $validator = $VALIDATORS{$class->_core_type}{$preserve} //= Perldantic::FFI::Validator->new(
        {type => $class->_core_type, preserve_empty_path => $preserve ? !!1 : !!0});
    return $validator->validate("$text");
}

# A value the core sent: its text is already normalised.
sub _from_wire ($class, $text) { bless {text => $text}, $class }

sub as_string ($self) { $self->{text} }

sub _parts ($self) {
    require Perldantic::FFI;
    return $self->{parts} //= Perldantic::FFI::url_parts($self);
}

sub scheme ($self)         { $self->_parts->{scheme} }
sub path ($self)           { $self->_parts->{path} }
sub query ($self)          { $self->_parts->{query} }
sub query_params ($self)   { $self->_parts->{query_params} }
sub fragment ($self)       { $self->_parts->{fragment} }
sub unicode_string ($self) { $self->_parts->{unicode_string} }
sub username ($self)       { $self->_parts->{username} }
sub password ($self)       { $self->_parts->{password} }
sub host ($self)           { $self->_parts->{host} }
sub unicode_host ($self)   { $self->_parts->{unicode_host} }
sub port ($self)           { $self->_parts->{port} }

# The URL as a URI object.
sub to_uri ($self) {
    require URI;
    return URI->new($self->{text});
}

# `Url.build`: the URL from its parts (credentials are not encoded, as in pydantic).
sub build ($class, %parts) {
    $class->_usage('build: scheme and host are required') if !defined $parts{scheme} || !defined $parts{host};
    my $url = "$parts{scheme}://" . _host_text(\%parts);
    return $class->new($url . _tail(\%parts));
}

sub _host_text ($parts) {
    my ($username, $password) = @$parts{qw(username password)};
    my $text = defined $username && defined $password ? "$username:$password\@"
        : defined $username ? "$username\@"
        : defined $password ? ":$password\@"
        : '';
    $text .= $parts->{host} // '';
    $text .= ":$parts->{port}" if defined $parts->{port};
    return $text;
}

sub _tail ($parts) {
    my $tail = '';
    $tail .= "/$parts->{path}"     if defined $parts->{path};
    $tail .= "?$parts->{query}"    if defined $parts->{query};
    $tail .= "#$parts->{fragment}" if defined $parts->{fragment};
    return $tail;
}

# The other operand as a URL of this class; text is parsed.
sub _other ($self, $other) {
    return $other if Scalar::Util::blessed($other) && $other->isa(ref $self);
    return eval { (ref $self)->new($other) } if defined $other && !ref $other;
    return undef;
}

# What pydantic compares: the parsed URL, where an empty path is `/`.
sub _compare_key ($self) {
    return $self->{key} //= (ref $self)->new($self->{text})->{text};
}

sub _equal ($self, $other) {
    $other = $self->_other($other) // return 0;
    return $self->_compare_key eq $other->_compare_key;
}

sub _compare ($self, $other) {
    my $url = $self->_other($other)
        // $self->_usage('compare: not a URL: ' . (ref $other || "'" . ($other // 'undef') . "'"));
    return $self->_compare_key cmp $url->_compare_key;
}

package Perldantic::MultiHostUrl {

    our @ISA = ('Perldantic::Url');

    sub _core_type ($class) { 'multi-host-url' }

    sub hosts ($self) { $self->_parts->{hosts} }

    # `MultiHostUrl.build`: `hosts` (hashes of username, password, host and port) or the
    # parts of a single host.
    sub build ($class, %parts) {
        $class->_usage('build: scheme is required') if !defined $parts{scheme};
        my $single = grep { defined $parts{$_} } qw(host username password port);
        $class->_usage('build: expected one of hosts or singular values to be set')
            if $parts{hosts} && $single;
        $class->_usage('build: expected either host or hosts to be set') if !$parts{hosts} && !defined $parts{host};
        my @hosts = $parts{hosts} ? @{$parts{hosts}} : (\%parts);
        return $class->new("$parts{scheme}://" . join(',', map { Perldantic::Url::_host_text($_) } @hosts)
                . Perldantic::Url::_tail(\%parts));
    }

    # pydantic compares multi-host URLs by their unicode strings.
    sub _compare_key ($self) { $self->unicode_string }

    for my $single (qw(username password host unicode_host port)) {
        no strict 'refs';
        *{$single} = sub ($self) {
            $self->_usage("$single: a multi-host URL has hosts, see hosts()");
        };
    }
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Url - URLs, the values the Url and MultiHostUrl types validate into

=head1 SYNOPSIS

    my $url = Perldantic::Url->new('https://user@example.com:8443/a?x=1#top');
    say "$url";              # https://user@example.com:8443/a?x=1#top
    say $url->host;          # example.com
    say $url->port;          # 8443
    my $uri = $url->to_uri;  # a URI object

    my $db = Perldantic::MultiHostUrl->new('postgres://u:p@h1:5432,h2/db');
    say $_->{host} for @{$db->hosts};

    my $built = Perldantic::Url->build(scheme => 'https', host => 'example.com', path => 'x');

=head1 DESCRIPTION

Modelled on pydantic's C<Url> and C<MultiHostUrl>, whose behaviour the core reproduces: C<new>
parses and normalises the text (C<< preserve_empty_path => 1 >> keeps an empty path empty) and
raises L<Perldantic::ValidationError> for invalid text; C<build> puts a URL together from its
parts. The value stringifies to its text (C<as_string>).

Accessors: C<scheme>, C<username>, C<password>, C<host>, C<unicode_host> (punycode decoded),
C<port> (or the scheme's default port), C<path>, C<query>, C<query_params> (a list of
C<[key, value]> pairs), C<fragment> and C<unicode_string>. A C<Perldantic::MultiHostUrl> has
C<hosts> (hash references with C<username>, C<password>, C<host> and C<port>) instead of the
single-host accessors.

URLs compare with C<eq>, C<ne> and C<cmp> as pydantic compares them, against URLs of the same
class or text. C<to_uri> converts to a L<URI> object (loaded on demand).

=cut
