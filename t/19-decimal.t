use v5.36;
use Test2::V0;

use Math::BigFloat;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(Decimal Num Int ArrayRef);
use Perldantic::Wire;

sub bf ($text) { Math::BigFloat->new($text) }

subtest 'wire' => sub {
    is Perldantic::Wire::encode([bf('1.50'), bf('1e100'), bf('-0.0000001')]),
        '[{"$decimal":"15e-1"},{"$decimal":"1e+100"},{"$decimal":"-1e-7"}]';
    is Perldantic::Wire::encode([Math::BigFloat->binf('-'), Math::BigFloat->bnan]),
        '[{"$decimal":"-Infinity"},{"$decimal":"NaN"}]';
    my $back = Perldantic::Wire::decode('[{"$decimal":"1.23E+4"},{"$decimal":"-Infinity"},{"$decimal":"sNaN"}]');
    isa_ok $_, 'Math::BigFloat' for @$back;
    is $back->[0]->bstr, '12300';
    ok $back->[1]->is_inf('-');
    ok $back->[2]->is_nan;

    my $kept = Perldantic::Wire::decode('{"$decimal":"2.50"}');
    is $kept->bstr, '2.5', 'Math::BigFloat drops trailing zeros';
    is Perldantic::Wire::encode($kept), '{"$decimal":"2.50"}', 'but the core gets its text back';
    $kept->badd(1);
    is Perldantic::Wire::encode($kept), '{"$decimal":"35e-1"}', 'until the value changes';

    # a rounded decimal keeps its scale, as Decimal.quantize does
    is Perldantic::Wire::encode([bf('278.996')->bfround(-2, 'common'), bf('12.5')->bround(5), bf('-3')->bfround(-1)]),
        '[{"$decimal":"279.00"},{"$decimal":"12.500"},{"$decimal":"-3.0"}]';
};

subtest 'the Decimal type' => sub {
    my $ta = Perldantic::TypeAdapter->new(Decimal);
    my $d = $ta->validate_python('3.14159265358979323846264338327950288');
    isa_ok $d, 'Math::BigFloat';
    is $d->bstr, '3.14159265358979323846264338327950288', 'every digit is kept';
    is $ta->validate_python(42)->bstr, '42';
    is $ta->validate_python(bf('0.1'))->bstr, '0.1';
    is $ta->validate_json('"1.5"')->bstr, '1.5';
    is $ta->dump_json(bf('1.50')), '"1.5"', 'JSON output is text, as in pydantic';
    is $ta->dump_json($ta->validate_python('19.990')), '"19.990"', 'validated decimals keep their exponent';
    is $ta->dump_python(bf('2.5'), mode => 'json'), '2.5';
    is $ta->json_schema, {anyOf => [{type => 'number'}, {type => 'string'}]};

    my $e = dies { $ta->validate_python('abc') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'decimal_parsing';
    $e = dies { $ta->validate_python('inf') };
    is $e->errors->[0]{type}, 'finite_number';

    my $money = Perldantic::TypeAdapter->new(Decimal->with(max_digits => 5, decimal_places => 2, ge => 0));
    is $money->validate_python('123.45')->bstr, '123.45';
    $e = dies { $money->validate_python('1.234') };
    is $e->errors->[0]{msg}, 'Decimal input should have no more than 2 decimal places';
    $e = dies { $money->validate_python('-1') };
    is $e->errors->[0]{type}, 'greater_than_equal';
    isa_ok $e->errors->[0]{ctx}{ge}, 'Math::BigFloat';

    my $cents = Perldantic::TypeAdapter->new(Decimal->with(multiple_of => bf('0.01')));
    ok lives { $cents->validate_python('0.30') };
    $e = dies { $cents->validate_python('0.305') };
    is $e->errors->[0]{msg}, 'Input should be a multiple of 0.01';

    my $strict = Perldantic::TypeAdapter->new(Decimal->with(strict => 1));
    $e = dies { $strict->validate_python('1.5') };
    is $e->errors->[0]{msg}, 'Input should be an instance of Decimal';
    is $strict->validate_python(bf('1.5'))->bstr, '1.5', 'Math::BigFloat objects pass strict mode';

    my $inf = Perldantic::TypeAdapter->new(Decimal->with(allow_inf_nan => 1));
    ok $inf->validate_python('-Infinity')->is_inf('-');
    $e = dies { Decimal->with(scale => 2) };
    is $e->message, "Constraint 'scale' does not apply to Decimal";
};

subtest 'Math::BigFloat for other number types' => sub {
    is Perldantic::TypeAdapter->new(Num)->validate_python(bf('0.25')), 0.25;
    is Perldantic::TypeAdapter->new(Int)->validate_python(bf('1e2')), 100;
    my $e = dies { Perldantic::TypeAdapter->new(Int)->validate_python(bf('1.5')) };
    is $e->errors->[0]{type}, 'int_from_float';
    is [map { $_->bstr } @{Perldantic::TypeAdapter->new(ArrayRef[Decimal])->validate_python([1, '2.5'])}],
        ['1', '2.5'];
};

package Test::Invoice {
    use Perldantic;
    use Perldantic::Types qw(Decimal Str);
    has item  => (is => 'ro', isa => Str);
    has total => (is => 'ro', isa => Decimal->with(decimal_places => 2));
}

package main;

subtest 'models' => sub {
    my $invoice = Test::Invoice->new(item => 'tea', total => '19.99');
    isa_ok $invoice->total, 'Math::BigFloat';
    is $invoice->model_dump_json, '{"item":"tea","total":"19.99"}';
    my $back = Test::Invoice->model_validate_json($invoice->model_dump_json);
    ok $back->total == $invoice->total, 'JSON round trip';
    is Test::Invoice->model_json_schema->{properties}{total},
        {anyOf => [{type => 'number'}, {type => 'string'}], title => 'Total'};
    my $e = dies { Test::Invoice->new(item => 'tea', total => '0.001') };
    is $e->errors->[0]{loc}, ['total'];
};

done_testing;
