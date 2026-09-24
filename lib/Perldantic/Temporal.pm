package Perldantic::Temporal;

use v5.36;

our $VERSION = '0.01';

use POSIX ();
use Scalar::Util ();

use Perldantic::Error;

# The base class of Perldantic::Date, ::Time, ::DateTime and ::Duration: values that stringify
# to their ISO 8601 text and compare with the usual operators, like Python's `datetime` types.
use overload
    '""'     => sub ($self, @) { $self->iso },
    '<=>'    => sub ($a, $b, $swap) { my $c = $a->_compare($b); $swap ? -$c : $c },
    'cmp'    => sub ($a, $b, $swap) { my $c = $a->_compare($b); $swap ? -$c : $c },
    '=='     => sub ($a, $b, @) { $a->_equal($b) },
    'eq'     => sub ($a, $b, @) { $a->_equal($b) },
    '!='     => sub ($a, $b, @) { !$a->_equal($b) },
    'ne'     => sub ($a, $b, @) { !$a->_equal($b) },
    bool     => sub { 1 },
    fallback => 1;

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

# The other operand as a value of the same kind (an ISO string is parsed).
sub _check_class ($self, $other) {
    $other = ref($self)->from_iso($other) if !ref $other;
    _usage("can't compare " . ref($self) . " with " . (ref $other || 'a string'))
        if !(Scalar::Util::blessed($other) && $other->isa('Perldantic::Temporal')
        && $other->_wire_tag eq $self->_wire_tag);
    return $other;
}

sub _equal ($self, $other) {
    $other = eval { $self->_check_class($other) } // return 0;
    return 0 if !$self->_aware_or_naive($other);
    return $self->_compare($other) == 0;
}

sub _compare ($self, $other) {
    $other = $self->_check_class($other);
    return $self->_compare_key cmp $other->_compare_key;
}

# Dates and durations have no timezone.
sub _aware_or_naive ($self, $other) { 1 }

sub _days_from_civil ($year, $month, $day) {
    # Howard Hinnant's days_from_civil: days since 1970-01-01
    $year -= $month <= 2 ? 1 : 0;
    my $era = POSIX::floor($year / 400);
    my $yoe = $year - $era * 400;
    my $doy = int((153 * ($month + ($month > 2 ? -3 : 9)) + 2) / 5) + $day - 1;
    my $doe = $yoe * 365 + int($yoe / 4) - int($yoe / 100) + $doy;
    return $era * 146097 + $doe - 719468;
}

sub _days_in_month ($year, $month) {
    return 29 if $month == 2 && ($year % 4 == 0 && ($year % 100 != 0 || $year % 400 == 0));
    return (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)[$month - 1];
}

sub _offset_iso ($offset) {
    my $sign = $offset < 0 ? '-' : '+';
    my $abs  = abs $offset;
    my $text = sprintf '%s%02d:%02d', $sign, int($abs / 3600), int($abs / 60) % 60;
    $text .= sprintf ':%02d', $abs % 60 if $abs % 60;
    return $text;
}

sub _parse_offset ($text, $class) {
    return undef if !defined $text || $text eq '';
    return 0 if $text eq 'Z';
    my ($sign, $h, $m, $s) = $text =~ /\A([+-])(\d\d):?(\d\d)(?::?(\d\d))?\z/
        or _usage("$class->from_iso: invalid UTC offset '$text'");
    my $offset = $h * 3600 + $m * 60 + ($s // 0);
    return $sign eq '-' ? -$offset : $offset;
}

sub _integer ($class, $name, $value, $min, $max) {
    _usage("$class->new: $name must be an integer from $min to $max, got " . ($value // 'undef'))
        if !defined $value || $value !~ /\A-?\d+\z/ || $value < $min || $value > $max;
    return 0 + $value;
}

package Perldantic::Date {
    our @ISA = ('Perldantic::Temporal');

    sub new ($class, %args) {
        my $year  = Perldantic::Temporal::_integer($class, 'year',  $args{year},  1, 9999);
        my $month = Perldantic::Temporal::_integer($class, 'month', $args{month}, 1, 12);
        my $day   = Perldantic::Temporal::_integer($class, 'day',   $args{day},   1, 31);
        Perldantic::Temporal::_usage(sprintf "$class->new: day %d is out of range for %04d-%02d", $day, $year, $month)
            if $day > Perldantic::Temporal::_days_in_month($year, $month);
        return bless {year => $year, month => $month, day => $day}, $class;
    }

    sub from_iso ($class, $iso) {
        my ($y, $m, $d) = ($iso // '') =~ /\A(\d{4})-(\d\d)-(\d\d)\z/
            or Perldantic::Temporal::_usage("$class->from_iso: not an ISO 8601 date: '" . ($iso // 'undef') . "'");
        return $class->new(year => $y, month => $m, day => $d);
    }

    sub year ($self)  { $self->{year} }
    sub month ($self) { $self->{month} }
    sub day ($self)   { $self->{day} }

    sub iso ($self) { sprintf '%04d-%02d-%02d', @$self{qw(year month day)} }
    sub _compare_key ($self) { $self->iso }
    sub _wire_tag ($self) {'$date'}

    sub to_datetime ($self) {
        require DateTime;
        return DateTime::->new(%$self, time_zone => 'floating');
    }

    sub to_time_moment ($self) {
        require Time::Moment;
        return Time::Moment->new(%$self, offset => 0);
    }
}

package Perldantic::Time {
    our @ISA = ('Perldantic::Temporal');

    sub new ($class, %args) {
        my %self = (
            hour        => Perldantic::Temporal::_integer($class, 'hour',        $args{hour}        // 0, 0, 23),
            minute      => Perldantic::Temporal::_integer($class, 'minute',      $args{minute}      // 0, 0, 59),
            second      => Perldantic::Temporal::_integer($class, 'second',      $args{second}      // 0, 0, 59),
            microsecond => Perldantic::Temporal::_integer($class, 'microsecond', $args{microsecond} // 0, 0, 999_999),
            tz_offset   => defined $args{tz_offset}
            ? Perldantic::Temporal::_integer($class, 'tz_offset', $args{tz_offset}, -86399, 86399)
            : undef,
        );
        return bless \%self, $class;
    }

    # `HH:MM[:SS[.ffffff]][Z|+HH:MM]` into new()'s arguments.
    sub _parse ($class, $text) {
        my ($h, $m, $s, $frac, $tz) = ($text // '')
            =~ /\A(\d\d):(\d\d)(?::(\d\d)(?:\.(\d{1,6}))?)?(Z|[+-]\d\d:?\d\d(?::?\d\d)?)?\z/
            or return;
        return (
            hour        => $h,
            minute      => $m,
            second      => $s // 0,
            microsecond => defined $frac ? substr($frac . '00000', 0, 6) + 0 : 0,
            tz_offset   => Perldantic::Temporal::_parse_offset($tz, $class),
        );
    }

    sub from_iso ($class, $iso) {
        my %args = $class->_parse($iso)
            or Perldantic::Temporal::_usage("$class->from_iso: not an ISO 8601 time: '" . ($iso // 'undef') . "'");
        return $class->new(%args);
    }

    sub hour ($self)        { $self->{hour} }
    sub minute ($self)      { $self->{minute} }
    sub second ($self)      { $self->{second} }
    sub microsecond ($self) { $self->{microsecond} }
    sub tz_offset ($self)   { $self->{tz_offset} }

    sub _time_iso ($self) {
        my $iso = sprintf '%02d:%02d:%02d', @$self{qw(hour minute second)};
        $iso .= sprintf '.%06d', $self->{microsecond} if $self->{microsecond};
        $iso .= Perldantic::Temporal::_offset_iso($self->{tz_offset}) if defined $self->{tz_offset};
        return $iso;
    }

    sub iso ($self) { $self->_time_iso }
    sub _wire_tag ($self) {'$time'}

    sub _micros ($self) {
        return (($self->{hour} * 60 + $self->{minute}) * 60 + $self->{second}) * 1_000_000 + $self->{microsecond};
    }

    sub _aware_or_naive ($self, $other) { !(defined $self->{tz_offset} xor defined $other->{tz_offset}) }

    sub _compare ($self, $other) {
        $other = $self->_check_class($other);
        Perldantic::Temporal::_usage("can't compare naive and aware times") if !$self->_aware_or_naive($other);
        my $utc = sub ($t) { $t->_micros - ($t->{tz_offset} // 0) * 1_000_000 };
        return $utc->($self) <=> $utc->($other);
    }
}

package Perldantic::DateTime {
    our @ISA = ('Perldantic::Time');

    sub new ($class, %args) {
        my $date = Perldantic::Date->new(map { $_ => $args{$_} } qw(year month day));
        my $time = Perldantic::Time->new(map { $_ => $args{$_} } qw(hour minute second microsecond tz_offset));
        return bless {%$date, %$time}, $class;
    }

    sub from_iso ($class, $iso) {
        my ($date, $time) = ($iso // '') =~ /\A(\d{4}-\d\d-\d\d)(?:[T ](.+))?\z/
            or Perldantic::Temporal::_usage("$class->from_iso: not an ISO 8601 datetime: '" . ($iso // 'undef') . "'");
        my %time = defined $time ? Perldantic::Time->_parse($time) : (hour => 0);
        Perldantic::Temporal::_usage("$class->from_iso: not an ISO 8601 datetime: '$iso'") if !%time;
        my %date;
        @date{qw(year month day)} = split /-/, $date;
        return $class->new(%date, %time);
    }

    sub year ($self)  { $self->{year} }
    sub month ($self) { $self->{month} }
    sub day ($self)   { $self->{day} }

    sub date ($self) { Perldantic::Date->new(map { $_ => $self->{$_} } qw(year month day)) }

    sub iso ($self) { sprintf('%04d-%02d-%02d', @$self{qw(year month day)}) . 'T' . $self->_time_iso }
    sub _wire_tag ($self) {'$datetime'}

    sub _micros ($self) {
        my $days = Perldantic::Temporal::_days_from_civil(@$self{qw(year month day)});
        return $days * 86_400_000_000 + $self->Perldantic::Time::_micros;
    }

    # Seconds since the Unix epoch; undef for a naive datetime.
    sub epoch ($self) {
        return undef if !defined $self->{tz_offset};
        return POSIX::floor(($self->_micros - $self->{tz_offset} * 1_000_000) / 1_000_000);
    }

    sub to_datetime ($self) {
        require DateTime;
        my $offset = $self->{tz_offset};
        return DateTime::->new(
            (map { $_ => $self->{$_} } qw(year month day hour minute second)),
            nanosecond => $self->{microsecond} * 1000,
            time_zone  => defined $offset ? DateTime::TimeZone->offset_as_string($offset) : 'floating',
        );
    }

    sub to_time_moment ($self) {
        require Time::Moment;
        Perldantic::Temporal::_usage('a naive datetime has no UTC offset; Time::Moment needs one')
            if !defined $self->{tz_offset};
        return Time::Moment->new(
            (map { $_ => $self->{$_} } qw(year month day hour minute second)),
            nanosecond => $self->{microsecond} * 1000,
            offset     => int($self->{tz_offset} / 60),
        );
    }
}

package Perldantic::Duration {
    our @ISA = ('Perldantic::Temporal');

    my %MICROS = (
        weeks        => 7 * 86_400_000_000,
        days         => 86_400_000_000,
        hours        => 3_600_000_000,
        minutes      => 60_000_000,
        seconds      => 1_000_000,
        milliseconds => 1_000,
        microseconds => 1,
    );

    # Like Python's timedelta(): any of weeks, days, hours, minutes, seconds, milliseconds and
    # microseconds, possibly fractional or negative, normalised to days, seconds, microseconds.
    sub new ($class, %args) {
        my $total = 0;
        for my $unit (sort keys %args) {
            my $per = $MICROS{$unit}
                // Perldantic::Temporal::_usage("$class->new: unknown unit '$unit'");
            Perldantic::Temporal::_usage("$class->new: $unit must be a number")
                if !Scalar::Util::looks_like_number($args{$unit});
            $total += $args{$unit} * $per;
        }
        return $class->_from_micros(POSIX::floor($total + 0.5));
    }

    sub _from_micros ($class, $total) {
        my $days = POSIX::floor($total / 86_400_000_000);
        my $rest = $total - $days * 86_400_000_000;
        return bless {
            days         => $days,
            seconds      => int($rest / 1_000_000),
            microseconds => $rest % 1_000_000,
        }, $class;
    }

    sub days ($self)         { $self->{days} }
    sub seconds ($self)      { $self->{seconds} }
    sub microseconds ($self) { $self->{microseconds} }

    sub _total_micros ($self) {
        return $self->{days} * 86_400_000_000 + $self->{seconds} * 1_000_000 + $self->{microseconds};
    }

    sub total_seconds ($self) { $self->_total_micros / 1_000_000 }

    sub _compare ($self, $other) {
        $other = $self->_check_class($other);
        return $self->_total_micros <=> $other->_total_micros;
    }

    sub _wire_tag ($self) {'$timedelta'}
    sub _wire_payload ($self) { [@$self{qw(days seconds microseconds)}] }

    sub from_iso ($class, $iso) {
        Perldantic::Temporal::_usage("$class->from_iso: durations are built with new(days => ..., seconds => ...)");
    }

    # ISO 8601 as the core writes durations (speedate): -P1Y2DT3H4M5.5S
    sub iso ($self) {
        my $total = $self->_total_micros;
        my $sign  = $total < 0 ? '-' : '';
        $total = abs $total;
        my $day    = int($total / 86_400_000_000);
        my $rest   = $total - $day * 86_400_000_000;
        my $second = int($rest / 1_000_000);
        my $micro  = $rest % 1_000_000;
        my $iso    = "${sign}P";
        $iso .= int($day / 365) . 'Y' if int($day / 365);
        $iso .= ($day % 365) . 'D' if $day % 365;
        if ($second || $micro) {
            $iso .= 'T';
            $iso .= int($second / 3600) . 'H' if int($second / 3600);
            $iso .= int(($second % 3600) / 60) . 'M' if int(($second % 3600) / 60);
            if ($second % 60 || $micro) {
                $iso .= $second % 60;
                $iso .= '.' . (sprintf('%06d', $micro) =~ s/0+\z//r) if $micro;
                $iso .= 'S';
            }
        }
        $iso .= 'T0S' if !$total;
        return $iso;
    }

    sub to_datetime_duration ($self) {
        require DateTime::Duration;
        my $total = $self->_total_micros;
        my $sign  = $total < 0 ? -1 : 1;
        $total = abs $total;
        return DateTime::Duration->new(
            seconds     => $sign * int($total / 1_000_000),
            nanoseconds => $sign * ($total % 1_000_000) * 1000,
        );
    }
}

sub _wire_payload ($self) { $self->iso }

# Output classes a model or type adapter can ask for (`temporal_class`).
our %CLASSES = map { $_ => 1 } qw(Perldantic DateTime Time::Moment);

# Convert the temporal values in validated data to `temporal_class`: DateTime turns dates,
# datetimes and durations into DateTime / DateTime::Duration objects; Time::Moment turns aware
# datetimes into Time::Moment objects. Other values stay as they are.
sub _convert_deep ($value, $class) {
    return $value if $class eq 'Perldantic';
    if (Scalar::Util::blessed $value) {
        if ($class eq 'DateTime') {
            return $value->to_datetime if $value->isa('Perldantic::Date') || $value->isa('Perldantic::DateTime');
            return $value->to_datetime_duration if $value->isa('Perldantic::Duration');
        }
        elsif ($value->isa('Perldantic::DateTime') && defined $value->tz_offset) {
            return $value->to_time_moment;
        }
        return $value;
    }
    return [map { _convert_deep($_, $class) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => _convert_deep($value->{$_}, $class) } keys %$value} if ref $value eq 'HASH';
    return $value;
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Temporal - dates, times, datetimes and durations

=head1 SYNOPSIS

    use Perldantic::Temporal;

    my $d  = Perldantic::Date->new(year => 2024, month => 2, day => 29);
    my $t  = Perldantic::Time->from_iso('12:13:14.5+01:00');
    my $dt = Perldantic::DateTime->from_iso('2022-06-08T12:13:14Z');
    my $p  = Perldantic::Duration->new(hours => 1, minutes => 30);

    say "$dt";                  # 2022-06-08T12:13:14+00:00
    say $dt->epoch;             # 1654690394
    say $dt->date;              # 2022-06-08
    say $p->total_seconds;      # 5400
    say "$p";                   # PT1H30M
    say 'later' if $dt > '2022-06-08T12:00:00Z';

=head1 DESCRIPTION

The values the C<Date>, C<Time>, C<DateTime> and C<Duration> types of L<Perldantic::Types>
validate into. They are modelled on Python's C<datetime> types, so they keep what pydantic
keeps: microseconds and a fixed UTC offset, but no time zone names.

=over

=item C<Perldantic::Date>

A calendar date.

=item C<Perldantic::Time>

A time of day, naive (without an offset) or aware (with a UTC offset).

=item C<Perldantic::DateTime>

A date and a time of day, naive or aware. It is a C<Perldantic::Time>.

=item C<Perldantic::Duration>

A length of time, Python's C<timedelta>: whole C<days>, C<seconds> (0 to 86399) and
C<microseconds> (0 to 999999). Negative durations have negative days.

=back

Every value stringifies to ISO 8601 (as Python's C<isoformat> does; durations as the core writes
them, such as C<P1DT2H>) and compares with C<< <=> >>, C<==>, C<!=>, C<cmp>, C<eq> and C<ne>,
against a value of the same class or its ISO text. As in Python, aware times compare in UTC, and
a naive value cannot be ordered against an aware one.

A model's C<< model_config temporal_class => 'DateTime' >> (or C<'Time::Moment'>, or the default
C<'Perldantic'>) converts the validated values of its fields: to L<DateTime> for dates and
datetimes and L<DateTime::Duration> for durations, or to L<Time::Moment> for aware datetimes;
other values stay as they are. L<Perldantic::TypeAdapter> takes the same setting in its
C<config>. Values of those classes are accepted as input too.

Invalid arguments raise C<Perldantic::UsageError> (see L<Perldantic::Error>).

=head1 Perldantic::Date

=head2 new(year => $year, month => $month, day => $day)

A date between 0001-01-01 and 9999-12-31.

=head2 from_iso($text)

A date from C<YYYY-MM-DD>.

=head2 year, month, day

The parts, as numbers.

=head2 iso

C<YYYY-MM-DD>, also what the date stringifies to.

=head2 to_datetime

The date as a floating L<DateTime> at midnight (DateTime is loaded on demand).

=head2 to_time_moment

The date as a L<Time::Moment> at midnight UTC (loaded on demand).

=head1 Perldantic::Time

=head2 new(hour => $h, minute => $m, second => $s, microsecond => $us, tz_offset => $offset)

A time of day; the parts default to 0. C<tz_offset> is the UTC offset in seconds east of UTC
(C<undef>, the default, for a naive time).

=head2 from_iso($text)

A time from C<HH:MM[:SS[.ffffff]]>, followed by C<Z> or an offset such as C<+01:00> for an aware
time.

=head2 hour, minute, second, microsecond, tz_offset

The parts; C<tz_offset> is C<undef> for a naive time.

=head2 iso

C<HH:MM:SS>, with C<.ffffff> when there are microseconds and the offset when the time is aware;
also what the time stringifies to.

=head1 Perldantic::DateTime

A C<Perldantic::DateTime> has the methods of C<Perldantic::Time> too.

=head2 new(year => ..., month => ..., day => ..., hour => ..., minute => ..., second => ..., microsecond => ..., tz_offset => ...)

A datetime; the time parts default to 0, and C<tz_offset> to C<undef> (naive).

=head2 from_iso($text)

A datetime from C<YYYY-MM-DD>, optionally followed by C<T> (or a space) and a time as
C<< Perldantic::Time->from_iso >> reads it.

=head2 year, month, day

The date parts.

=head2 date

The date, a C<Perldantic::Date>.

=head2 iso

C<YYYY-MM-DDTHH:MM:SS>, with microseconds and the offset as for times; also what the datetime
stringifies to.

=head2 epoch

Whole seconds since the Unix epoch, or C<undef> for a naive datetime.

=head2 to_datetime

The datetime as a L<DateTime> (loaded on demand), with its offset as the time zone, or floating
when it is naive.

=head2 to_time_moment

The datetime as a L<Time::Moment> (loaded on demand). A naive datetime has no offset to give it
and raises C<Perldantic::UsageError>.

=head1 Perldantic::Duration

=head2 new(%units)

A duration of any of C<weeks>, C<days>, C<hours>, C<minutes>, C<seconds>, C<milliseconds> and
C<microseconds>, which may be fractional or negative, as Python's C<timedelta()> takes them.
The sum is rounded to whole microseconds.

=head2 from_iso

Not supported: it raises C<Perldantic::UsageError>. Build durations with C<new>.

=head2 days, seconds, microseconds

The normalised parts: C<seconds> is 0 to 86399 and C<microseconds> 0 to 999999, so a negative
duration has negative C<days>.

=head2 total_seconds

The length in seconds, with microseconds as a fraction.

=head2 iso

ISO 8601 as the core writes durations, such as C<-P1Y2DT3H4M5.5S> or C<PT0S>; also what the
duration stringifies to.

=head2 to_datetime_duration

The duration as a L<DateTime::Duration> of seconds and nanoseconds (loaded on demand).

=head1 SEE ALSO

L<Perldantic::Types>, L<Perldantic>, L<DateTime>, L<Time::Moment>

=cut
