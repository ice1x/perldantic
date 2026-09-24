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

    use Perldantic::Url;

    my $url = Perldantic::Url->new('https://user@example.com:8443/a?x=1#top');
    say "$url";              # https://user@example.com:8443/a?x=1#top
    say $url->host;          # example.com
    say $url->port;          # 8443
    say $url->path;          # /a

    my $db = Perldantic::MultiHostUrl->new('postgres://u:p@h1:5432,h2/db');
    say $_->{host} for @{$db->hosts};                  # h1, h2

    my $built = Perldantic::Url->build(scheme => 'https', host => 'example.com', path => 'x');
    say 'same' if $built eq 'https://example.com/x';

=head1 DESCRIPTION

URLs modelled on pydantic's C<Url> and C<MultiHostUrl>; the core parses and normalises them
exactly as pydantic does. The L<Url|Perldantic::Types/Url> and
L<MultiHostUrl|Perldantic::Types/MultiHostUrl> types validate text into these objects.

A URL stringifies to its normalised text and compares with C<eq>, C<ne> and C<cmp> as pydantic
compares URLs, against URLs of the same class or against text (which is parsed first). The parts
are computed by the core on first use and cached.

C<Perldantic::MultiHostUrl> is a C<Perldantic::Url> with several hosts, such as a database URL
naming a cluster. It has L</hosts> instead of the single-host accessors, which raise
L<Perldantic::UsageError> on it.

=head1 METHODS

=head2 new($text, preserve_empty_path => $bool)

Parses and normalises C<$text>. An empty path becomes C</> unless C<preserve_empty_path> is
true. Invalid text raises L<Perldantic::ValidationError>, with pydantic's error codes.

=head2 build(%parts)

Puts a URL together from C<scheme>, C<username>, C<password>, C<host>, C<port>, C<path>,
C<query> and C<fragment>, as pydantic's C<Url.build> does; C<scheme> and C<host> are required.
For a C<Perldantic::MultiHostUrl>, C<< hosts => [\%host, ...] >> (each with C<username>,
C<password>, C<host> and C<port>) replaces the single-host parts.

=head2 as_string

The normalised text, also what the URL stringifies to.

=head2 unicode_string

The text with the host decoded from punycode.

=head2 scheme, username, password, host, unicode_host, port

The scheme, the credentials (or C<undef>), the host (C<unicode_host>: decoded from punycode)
and the port, or the scheme's default port.

=head2 path, query, query_params, fragment

The path, the query string, the query as a list of C<[$key, $value]> pairs, and the fragment;
each is C<undef> when absent.

=head2 hosts

For a C<Perldantic::MultiHostUrl>: the hosts, as hash references with C<username>, C<password>,
C<host> and C<port>.

=head2 to_uri

The URL as a L<URI> object (loaded on demand).

=head1 SEE ALSO

L<Perldantic::Types/Url>, L<Perldantic>

=cut
