use v5.36;
use Test2::V0;

use Perldantic::Temporal;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(Date Time DateTime Duration ArrayRef Maybe);

subtest 'value classes' => sub {
    my $d = Perldantic::Date->new(year => 2024, month => 2, day => 29);
    is "$d", '2024-02-29', 'dates stringify to ISO 8601';
    is [$d->year, $d->month, $d->day], [2024, 2, 29];
    ok $d < Perldantic::Date->from_iso('2024-03-01'), 'dates compare';
    ok $d == Perldantic::Date->from_iso('2024-02-29');
    my $e = dies { Perldantic::Date->new(year => 2023, month => 2, day => 29) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'Perldantic::Date->new: day 29 is out of range for 2023-02';

    my $t = Perldantic::Time->from_iso('12:13:14.000500+01:00');
    is [$t->hour, $t->minute, $t->second, $t->microsecond, $t->tz_offset], [12, 13, 14, 500, 3600];
    is "$t", '12:13:14.000500+01:00';
    is Perldantic::Time->new(hour => 9)->iso, '09:00:00';
    ok Perldantic::Time->from_iso('11:13:14.0005Z') == $t, 'aware times compare in UTC';
    ok !(Perldantic::Time->from_iso('12:13:14.0005') == $t), 'naive and aware times are never equal';
    $e = dies { my $x = Perldantic::Time->from_iso('12:00') < $t };
    is $e->message, "can't compare naive and aware times";

    my $dt = Perldantic::DateTime->from_iso('2022-06-08T12:13:14+01:00');
    is "$dt", '2022-06-08T12:13:14+01:00';
    is $dt->date, Perldantic::Date->from_iso('2022-06-08');
    is $dt->epoch, 1654686794, 'aware datetimes have an epoch';
    ok $dt == Perldantic::DateTime->from_iso('2022-06-08T11:13:14Z');
    ok !defined Perldantic::DateTime->from_iso('2022-06-08T12:13:14')->epoch, 'naive ones do not';

    my $p = Perldantic::Duration->new(days => 1, hours => 2, seconds => -1, microseconds => 5);
    is [$p->days, $p->seconds, $p->microseconds], [1, 7199, 5], "timedelta's normalised fields";
    is $p->total_seconds, 93599.000005;
    is Perldantic::Duration->new(seconds => -1)->iso, '-PT1S';
    is Perldantic::Duration->new(days => 1, seconds => 1)->iso, 'P1DT1S';
    is Perldantic::Duration->new(seconds => 1.5)->iso, 'PT1.5S';
    is Perldantic::Duration->new->iso, 'PT0S';
    ok Perldantic::Duration->new(hours => 1) > Perldantic::Duration->new(minutes => 59);
};

subtest 'types validate through the core' => sub {
    my $v = sub ($type, $input, %opts) { Perldantic::TypeAdapter->new($type)->validate($input, %opts) };
    my $d = $v->(Date, '2022-06-08');
    isa_ok $d, 'Perldantic::Date';
    is "$d", '2022-06-08';
    is "@{[ $v->(DateTime, '2022-06-08 12:00+02:00') ]}", '2022-06-08T12:00:00+02:00';
    is "@{[ $v->(DateTime, 1654646400) ]}", '2022-06-08T00:00:00+00:00', 'Unix timestamps';
    is "@{[ $v->(Time, '12:30') ]}", '12:30:00';
    my $p = $v->(Duration, 'P1DT2H');
    isa_ok $p, 'Perldantic::Duration';
    is [$p->days, $p->seconds], [1, 7200];
    is $v->(Duration, 90)->total_seconds, 90, 'numbers are seconds';

    my $e = dies { $v->(Date, 'nope') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'date_from_datetime_parsing';
    $e = dies { $v->(Date->with(lt => '2000-01-01'), '2022-06-08') };
    is $e->errors->[0]{msg}, 'Input should be less than 2000-01-01';
    $e = dies { $v->(DateTime->with(tz_constraint => 'aware'), '2022-06-08T12:00') };
    is $e->errors->[0]{type}, 'timezone_aware';
    $e = dies { $v->(Date, '2022-06-08', strict => 1) };
    is $e->errors->[0]{type}, 'date_type', 'strict mode wants date objects';
    is "@{[ $v->(Date, Perldantic::Date->from_iso('2022-06-08'), strict => 1) ]}", '2022-06-08';
};

subtest 'DateTime and Time::Moment objects are accepted' => sub {
    my $v = sub ($type, $input) { Perldantic::TypeAdapter->new($type)->validate($input) };
    SKIP: {
        skip 'DateTime is not installed', 4 if !eval { require DateTime; 1 };
        my $aware = DateTime::->new(year => 2022, month => 6, day => 8, hour => 12, nanosecond => 5000,
            time_zone => '+0100');
        is "@{[ $v->(DateTime, $aware) ]}", '2022-06-08T12:00:00.000005+01:00';
        my $floating = DateTime::->new(year => 2022, month => 6, day => 8);
        is "@{[ $v->(DateTime, $floating) ]}", '2022-06-08T00:00:00', 'floating DateTimes are naive';
        is "@{[ $v->(Date, $floating) ]}", '2022-06-08', 'a DateTime at midnight is a date';
        require DateTime::Duration;
        is $v->(Duration, DateTime::Duration->new(hours => 1, nanoseconds => 1000))->total_seconds, 3600.000001;
    }
    SKIP: {
        skip 'Time::Moment is not installed', 1 if !eval { require Time::Moment; 1 };
        my $tm = Time::Moment->new(year => 2022, month => 6, day => 8, hour => 1, offset => -300);
        is "@{[ $v->(DateTime, $tm) ]}", '2022-06-08T01:00:00-05:00';
    }
};

subtest 'calling a class that shares a type name' => sub {
    my $e = dies { my $x = DateTime->new(year => 2022) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message,
        'DateTime->new(...) called the Perldantic type DateTime; to call the class, write DateTime::->new(...)';
    $e = dies { my $x = DateTime->now };
    is $e->message,
        'DateTime->now(...) called the Perldantic type DateTime; to call the class, write DateTime::->now(...)';
};

subtest 'conversions' => sub {
    SKIP: {
        skip 'DateTime is not installed', 4 if !eval { require DateTime; 1 };
        my $dt = Perldantic::DateTime->from_iso('2022-06-08T12:13:14.5+01:00')->to_datetime;
        isa_ok $dt, 'DateTime';
        is $dt->iso8601 . ' ' . $dt->time_zone->name, '2022-06-08T12:13:14 +0100';
        is $dt->nanosecond, 500_000_000;
        is Perldantic::Date->from_iso('2022-06-08')->to_datetime->time_zone->name, 'floating';
    }
    SKIP: {
        skip 'Time::Moment is not installed', 2 if !eval { require Time::Moment; 1 };
        is Perldantic::DateTime->from_iso('2022-06-08T12:13:14Z')->to_time_moment->to_string, '2022-06-08T12:13:14Z';
        my $e = dies { Perldantic::DateTime->from_iso('2022-06-08T12:13:14')->to_time_moment };
        is $e->message, 'a naive datetime has no UTC offset; Time::Moment needs one';
    }
};

package Test::Event {
    use Perldantic;
    has at      => (is => 'ro', isa => DateTime, required => 1);
    has on      => (is => 'ro', isa => Maybe[Date]);
    has lasts   => (is => 'ro', isa => Duration, default => sub { Perldantic::Duration->new(hours => 1) });
    has reminds => (is => 'ro', isa => ArrayRef[Time], default => sub { [] });
}

package Test::EventDT {
    use Perldantic;
    model_config temporal_class => 'DateTime';
    has at => (is => 'ro', isa => DateTime);
    has on => (is => 'ro', isa => Date);
}

package main;

subtest 'models' => sub {
    my $ev = Test::Event->new(at => '2022-06-08T12:00Z', on => '2022-06-08', reminds => ['09:00']);
    isa_ok $ev->at, 'Perldantic::DateTime';
    isa_ok $ev->reminds->[0], 'Perldantic::Time';
    is $ev->lasts->total_seconds, 3600;
    is $ev->model_dump_json,
        '{"at":"2022-06-08T12:00:00Z","on":"2022-06-08","lasts":"PT1H","reminds":["09:00:00"]}';
    my $dump = $ev->model_dump;
    isa_ok $dump->{at}, 'Perldantic::DateTime';
    is $ev->model_dump(mode => 'json')->{on}, '2022-06-08';
    my $back = Test::Event->model_validate_json($ev->model_dump_json);
    ok $back->at == $ev->at, 'JSON round trip';
    is Test::Event->model_json_schema->{properties}{at}, {type => 'string', format => 'date-time', title => 'At'};
    SKIP: {
        skip 'DateTime is not installed', 2 if !eval { require DateTime; 1 };
        my $dt = Test::EventDT->new(at => '2022-06-08T12:00+02:00', on => '2022-06-08');
        isa_ok $dt->at, 'DateTime';
        is $dt->on->ymd, '2022-06-08', "temporal_class => 'DateTime' converts every temporal field";
    }
    my $e = dies { package Test::BadTC; use Perldantic; model_config temporal_class => 'Nope' };
    is $e->message, "model_config: temporal_class must be Perldantic, DateTime or Time::Moment, got 'Nope'";
};

done_testing;
