use v5.36;
use Test2::V0;

ok require Perldantic, 'Perldantic loads';
like $Perldantic::VERSION, qr/^\d+\.\d+$/, 'Perldantic has a numeric version';

done_testing;
