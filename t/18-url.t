use v5.36;
use Test2::V0;

use Perldantic::Url;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(Url MultiHostUrl ArrayRef);
use Perldantic::Wire;

subtest 'Perldantic::Url' => sub {
    my $u = Perldantic::Url->new('https://user:pw@xn--mnchen-3ya.de:8443/a/b?x=1&y=a%20b&x=2#frag');
    isa_ok $u, 'Perldantic::Url';
    is "$u", 'https://user:pw@xn--mnchen-3ya.de:8443/a/b?x=1&y=a%20b&x=2#frag';
    is [$u->scheme, $u->username, $u->password, $u->host, $u->port], ['https', 'user', 'pw', 'xn--mnchen-3ya.de', 8443];
    is $u->unicode_host, "m\x{fc}nchen.de";
    is [$u->path, $u->query, $u->fragment], ['/a/b', 'x=1&y=a%20b&x=2', 'frag'];
    is $u->query_params, [[x => 1], [y => 'a b'], [x => 2]];

    my $plain = Perldantic::Url->new('ftp://example.com');
    is "$plain", 'ftp://example.com/', 'normalised like pydantic';
    is [$plain->username, $plain->port, $plain->path, $plain->query], [undef, 21, '/', undef];
    is "@{[ Perldantic::Url->new('https://example.com?q=1', preserve_empty_path => 1) ]}",
        'https://example.com?q=1';

    ok $plain eq Perldantic::Url->new('ftp://example.com/'), 'equal URLs';
    ok $plain eq 'ftp://example.com', 'and equal to text that parses to the same URL';
    ok Perldantic::Url->new('https://a.com') lt Perldantic::Url->new('https://b.com');
    ok(Perldantic::Url->new('https://example.com') eq Perldantic::Url->new('https://example.com', preserve_empty_path => 1),
        'an empty path compares equal to /, as in pydantic');

    my $built = Perldantic::Url->build(scheme => 'https', host => 'example.com', port => 8080, path => 'x', query => 'a=1');
    is "$built", 'https://example.com:8080/x?a=1';

    my $e = dies { Perldantic::Url->new('not a url') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'url_parsing';
    $e = dies { Perldantic::Url->build(host => 'x') };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'Perldantic::Url->build: scheme and host are required';

    my $uri = $u->to_uri;
    isa_ok $uri, 'URI';
    is $uri->host, 'xn--mnchen-3ya.de';
    is $uri->port, 8443;
};

subtest 'Perldantic::MultiHostUrl' => sub {
    my $m = Perldantic::MultiHostUrl->new('postgres://u:p@h1:5432,h2,h3:1/db?x=1');
    is "$m", 'postgres://u:p@h1:5432,h2,h3:1/db?x=1';
    is $m->scheme, 'postgres';
    is $m->hosts, [
        {username => 'u', password => 'p', host => 'h1', port => 5432},
        {username => undef, password => undef, host => 'h2', port => undef},
        {username => undef, password => undef, host => 'h3', port => 1},
    ];
    is [$m->path, $m->query, $m->fragment], ['/db', 'x=1', undef];
    is Perldantic::MultiHostUrl->new('https://xn--mnchen-3ya.de,b.com/p')->unicode_string, "https://m\x{fc}nchen.de,b.com/p";
    ok $m eq 'postgres://u:p@h1:5432,h2,h3:1/db?x=1';
    is "@{[ Perldantic::MultiHostUrl->build(scheme => 'redis', hosts => [{host => 'a', port => 1}, {host => 'b'}], path => '0') ]}",
        'redis://a:1,b/0';
    my $e = dies { Perldantic::MultiHostUrl->new('redis://a,,b') };
    is $e->errors->[0]{msg}, 'Input should be a valid URL, empty host';
};

subtest 'wire' => sub {
    my $u = Perldantic::Url->new('https://example.com');
    is Perldantic::Wire::encode([$u]), '[{"$url":"https://example.com/"}]';
    my $back = Perldantic::Wire::decode('{"$multi_host_url":"redis://a,b"}');
    isa_ok $back, 'Perldantic::MultiHostUrl';
    is "$back", 'redis://a,b';
    require URI;
    is Perldantic::Wire::encode([URI->new('https://example.com')]), '["https://example.com"]', 'URI objects are text';
};

subtest 'the Url and MultiHostUrl types' => sub {
    my $ta = Perldantic::TypeAdapter->new(Url);
    my $u = $ta->validate('https://example.com/x');
    isa_ok $u, 'Perldantic::Url';
    is $ta->validate(URI->new('https://example.com'))->as_string, 'https://example.com/', 'URI objects';
    is $ta->validate($u)->as_string, 'https://example.com/x', 'Url objects';
    is $ta->dump_json($u), '"https://example.com/x"';
    is $ta->dump($u, mode => 'json'), 'https://example.com/x';
    is $ta->json_schema, {type => 'string', format => 'uri', minLength => 1};

    my $https = Perldantic::TypeAdapter->new(Url->with(allowed_schemes => ['https'], max_length => 30));
    my $e = dies { $https->validate('ftp://example.com') };
    is $e->errors->[0]{msg}, "URL scheme should be 'https'";
    $e = dies { $https->validate('https://example.com/' . 'x' x 30) };
    is $e->errors->[0]{type}, 'url_too_long';

    my $defaults = Perldantic::TypeAdapter->new(
        Url->with(default_host => 'localhost', default_port => 8000, default_path => '/api', host_required => 1));
    is $defaults->validate('redis://')->as_string, 'redis://localhost:8000/api';
    is Perldantic::TypeAdapter->new(Url->with(preserve_empty_path => 1))->validate('https://ex.com')->as_string,
        'https://ex.com';

    my $multi = Perldantic::TypeAdapter->new(MultiHostUrl->with(allowed_schemes => ['redis']));
    my $m = $multi->validate('redis://a:1,b:2/0');
    isa_ok $m, 'Perldantic::MultiHostUrl';
    is scalar @{$m->hosts}, 2;
    is Perldantic::TypeAdapter->new(MultiHostUrl)->json_schema, {type => 'string', format => 'multi-host-uri', minLength => 1};
    $e = dies { MultiHostUrl->with(pattern => 'x') };
    is $e->message, "Constraint 'pattern' does not apply to MultiHostUrl";

    is [map {"$_"} @{Perldantic::TypeAdapter->new(ArrayRef[Url])->validate(['http://a.com', 'http://b.com'])}],
        ['http://a.com/', 'http://b.com/'], 'nested';
};

package Test::Service {
    use Perldantic;
    use Perldantic::Types qw(Url MultiHostUrl);
    model_config url_preserve_empty_path => 1;
    has homepage => (is => 'ro', isa => Url);
    has cache    => (is => 'ro', isa => MultiHostUrl->with(allowed_schemes => ['redis']));
}

package main;

subtest 'models' => sub {
    my $s = Test::Service->new(homepage => 'https://example.com', cache => 'redis://a,b/0');
    isa_ok $s->homepage, 'Perldantic::Url';
    is $s->model_dump_json, '{"homepage":"https://example.com","cache":"redis://a,b/0"}',
        'url_preserve_empty_path from the model config';
    my $back = Test::Service->model_validate_json($s->model_dump_json);
    ok $back->cache eq $s->cache, 'JSON round trip';
    is Test::Service->model_json_schema->{properties}{homepage},
        {type => 'string', format => 'uri', minLength => 1, title => 'Homepage'};
    my $e = dies { Test::Service->new(homepage => 'https://example.com', cache => 'http://a') };
    is $e->errors->[0]{loc}, ['cache'];
};

done_testing;
