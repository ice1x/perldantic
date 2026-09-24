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
    is Perldantic::Wire::encode([1.7976931348623157e308, 0.1, 0.1 + 0.2, 1e20, -2.5e-12, 5e-324]),
        '[1.7976931348623157e+308,0.1,0.30000000000000004,1e+20,-2.5e-12,5e-324]',
        'floats keep full precision in their shortest form';
    my $n = 7;
    my $s = "$n";
    is Perldantic::Wire::encode(["5", 5, $s, 18446744073709551615, -9223372036854775808]),
        '["5",5,"7",18446744073709551615,-9223372036854775808]', 'strings and integers keep their kind';
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
    is Perldantic::Wire::encode(ordered(tuple(1), 2, 3.5, 'x')), '{"$dict":[[{"$tuple":[1]},2],[3.5,"x"]]}',
        'keys may be any value';
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
    is Perldantic::Wire::decode('{"$dict":[[{"$tuple":[1,2]},"t"]]}'), {'{"$tuple":[1,2]}' => 't'},
        'a key that is not a string or number is keyed by its wire JSON';
    is Perldantic::Wire::decode('{"$dict":[[null,1]]}'), {'' => 1}, 'a null key is the empty string';
    is $data->{u}, "caf\x{e9}", 'strings are decoded text';
    isa_ok $data->{big}, 'Math::BigInt';
    my $floats = Perldantic::Wire::decode('[0.01, 1e-12, 1.7976931348623157e+308, 9.223372036854776e+18]');
    is [map { ref } @$floats], ['', '', '', ''], 'floats are plain numbers, never Math::BigFloat';
    is $floats->[0], 0.01;
    is $floats->[1], 1e-12;
    is "$data->{big}", '123456789012345678901234567890';
};

subtest 'dates, times and durations round-trip as Perldantic::Temporal values' => sub {
    my $data = Perldantic::Wire::decode(
        '[{"$date":"2022-06-08"},{"$time":"12:13:14.000001+01:00"},'
            . '{"$datetime":"2022-06-08T12:13:14+01:00"},{"$timedelta":[-1,86399,5]}]');
    isa_ok $data->[0], 'Perldantic::Date';
    is $data->[0]->iso, '2022-06-08';
    isa_ok $data->[1], 'Perldantic::Time';
    is $data->[1]->iso, '12:13:14.000001+01:00';
    isa_ok $data->[2], 'Perldantic::DateTime';
    is "$data->[2]", '2022-06-08T12:13:14+01:00', 'they stringify to their ISO text';
    isa_ok $data->[3], 'Perldantic::Duration';
    is [$data->[3]->days, $data->[3]->seconds, $data->[3]->microseconds], [-1, 86399, 5],
        "Python's normalised timedelta fields";
    is Perldantic::Wire::encode($data),
        '[{"$date":"2022-06-08"},{"$time":"12:13:14.000001+01:00"},'
        . '{"$datetime":"2022-06-08T12:13:14+01:00"},{"$timedelta":[-1,86399,5]}]';
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

subtest 'enum members round-trip' => sub {
    my $member = Perldantic::Wire::decode(
        '{"$enum":{"class":"Color","name":"RED","value":{"$tuple":[1]},"mixin":"int","str_is_value":true}}');
    isa_ok $member, 'Perldantic::Wire::Enum';
    is $member->class, 'Color';
    is $member->name, 'RED';
    is $member->value, [1];
    is $member->mixin, 'int';
    ok $member->str_is_value;
    is Perldantic::Wire::encode($member),
        '{"$enum":{"class":"Color","mixin":"int","name":"RED","str_is_value":true,"value":[1]}}';
    my $plain = Perldantic::Wire::Enum->new(class => 'Color', name => 'RED', value => 1);
    is Perldantic::Wire::encode($plain), '{"$enum":{"class":"Color","name":"RED","value":1}}',
        'mixin and str_is_value may be left out';
};

subtest 'code references are host functions' => sub {
    sub check_value ($value) { $value }
    my $json = Perldantic::Wire::encode([\&check_value]);
    like $json, qr/\A\[\{"\$function":\{"id":\d+,"name":"check_value"\}\}\]\z/;
    my ($id) = $json =~ /"id":(\d+)/;
    ref_is Perldantic::Wire::function($id), \&check_value, 'known by id';
    ref_is Perldantic::Wire::decode(qq({"\$function":{"id":$id,"name":"check_value"}})), \&check_value,
        'decoded back into the code reference';
    like Perldantic::Wire::encode(sub {1}), qr/"name":"__ANON__"/;
    my $e = dies { Perldantic::Wire::decode('{"$function":{"id":1,"name":"gone"}}') };
    isa_ok $e, 'Perldantic::InternalError';
};

subtest 'unsupported values are usage errors' => sub {
    for my $bad (\1, bless({}, 'Some::Class')) {
        my $e = dies { Perldantic::Wire::encode([$bad]) };
        isa_ok $e, 'Perldantic::UsageError';
        like $e->message, qr/^Cannot pass .+ to the core: .+ has no wire form$/;
    }
    my $e = dies { Perldantic::Wire::decode('{') };
    isa_ok $e, 'Perldantic::InternalError';
    like $e->message, qr/^Malformed JSON from the core: /;
};

done_testing;
