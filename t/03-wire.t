use v5.36;
use Test2::V0;
no warnings 'experimental::builtin';
use Math::BigInt;

use Perldantic::Wire qw(tuple set bytes ordered);

my $inf = 9**9**9;

subtest 'plain values are written as JSON' => sub {
    is Perldantic::Wire::encode(undef), 'null';
    is Perldantic::Wire::encode([1, 2.5, 'x', !!1, !!0, undef]), '[1,2.5,"x",true,false,null]';
    is Perldantic::Wire::encode(3.0), '3.0', 'integral floats stay floats';
    is Perldantic::Wire::encode({b => 1, a => {}}), '{"a":{},"b":1}', 'keys are sorted';
    is Perldantic::Wire::encode("caf\x{e9}"), qq{"caf\xc3\xa9"}, 'text is UTF-8 encoded';
    is Perldantic::Wire::encode(Math::BigInt->new('123456789012345678901234567890')),
        '123456789012345678901234567890', 'big integers stay numbers';
};

subtest 'values JSON cannot express are tagged' => sub {
    is Perldantic::Wire::encode(tuple(1, 'a')), '{"$tuple":[1,"a"]}';
    is Perldantic::Wire::encode(set(1)), '{"$set":[1]}';
    is Perldantic::Wire::encode(bytes("hi")), '{"$bytes":"aGk="}';
    is Perldantic::Wire::encode([$inf, -$inf]), '[{"$float":"inf"},{"$float":"-inf"}]';
    is Perldantic::Wire::encode({'$ref' => 1, a => 2}), '{"$dict":[["$ref",1],["a",2]]}',
        'a hash with a $-key is a list of pairs';
    is Perldantic::Wire::encode('inf'), '"inf"', 'the string "inf" stays a string';
    is Perldantic::Wire::encode(
        Perldantic::Wire::Model->new(class => 'My::Point', fields => {x => 1})),
        '{"$model":{"class":"My::Point","extra":null,"fields":{"x":1},"fields_set":["x"]}}';
};

subtest 'ordered dicts and objects with a wire form' => sub {
    is Perldantic::Wire::encode(ordered(b => 1, a => tuple(2))), '{"$dict":[["b",1],["a",{"$tuple":[2]}]]}',
        'ordered() keeps the given key order';
    package Test::WireObject { sub new { bless {}, shift } sub _perldantic_wire { Perldantic::Wire::tuple(7) } }
    is Perldantic::Wire::encode([Test::WireObject->new]), '[{"$tuple":[7]}]',
        'objects can provide their wire form';
    my $e = dies { ordered('a') };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'ordered() takes key => value pairs';
};

subtest 'wire JSON is decoded into Perl data' => sub {
    my $data = Perldantic::Wire::decode(
        '{"a":[1,2.5,"x",true,false,null],"t":{"$tuple":[1]},"s":{"$set":["a"]},'
            . '"b":{"$bytes":"aGk="},"f":{"$float":"-inf"},"d":{"$dict":[[1,"one"],["$x",2]]},'
            . qq{"u":"caf\xc3\xa9","big":123456789012345678901234567890}
            . '}');
    is $data->{a}, [1, 2.5, 'x', T(), F(), undef];
    ok builtin::is_bool($data->{a}[3]), 'booleans are native Perl booleans';
    is $data->{t}, [1], 'tuples become array references';
    is $data->{s}, ['a'], 'sets become array references';
    is $data->{b}, 'hi', 'bytes become byte strings';
    is $data->{f}, -$inf;
    is $data->{d}, {1 => 'one', '$x' => 2};
    is $data->{u}, "caf\x{e9}", 'strings are decoded text';
    isa_ok $data->{big}, 'Math::BigInt';
    is "$data->{big}", '123456789012345678901234567890';
};

subtest 'models round-trip' => sub {
    my $model = Perldantic::Wire::decode(
        '{"$model":{"class":"M","fields":{"a":{"$tuple":[1]}},"fields_set":["a"],"extra":{"z":1}}}');
    isa_ok $model, 'Perldantic::Wire::Model';
    is $model->class, 'M';
    is $model->fields, {a => [1]};
    is $model->fields_set, ['a'];
    is $model->extra, {z => 1};
    is Perldantic::Wire::encode($model),
        '{"$model":{"class":"M","extra":{"z":1},"fields":{"a":[1]},"fields_set":["a"]}}';
};

subtest 'unsupported values are usage errors' => sub {
    for my $bad (sub {1}, \1, bless({}, 'Some::Class')) {
        my $e = dies { Perldantic::Wire::encode([$bad]) };
        isa_ok $e, 'Perldantic::UsageError';
        like $e->message, qr/^Cannot pass .+ to the core: .+ has no wire form$/;
    }
    my $e = dies { Perldantic::Wire::decode('{') };
    isa_ok $e, 'Perldantic::InternalError';
    like $e->message, qr/^Malformed JSON from the core: /;
};

done_testing;
