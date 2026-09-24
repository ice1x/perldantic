use v5.36;
use Test2::V0;

# Every Perl example in docs/MIGRATING_FROM_PYDANTIC.md runs as written.

use File::Temp qw(tempfile);

my $guide = 'docs/MIGRATING_FROM_PYDANTIC.md';
open my $fh, '<:encoding(UTF-8)', $guide or die "$guide: $!";
my $text = do { local $/; <$fh> };

my @blocks;
while ($text =~ /^```perl\n(.*?)^```$/msg) {
    my $line = 1 + (substr($text, 0, $-[1]) =~ tr/\n//);
    push @blocks, [$line, $1];
}
ok scalar @blocks >= 10, 'the guide has Perl examples';
like $text, qr/^```python\n/m, 'and the pydantic code they replace';

my $lib = -d 'blib/lib' ? 'blib/lib' : 'lib';
my $arch = -d 'blib/arch' ? 'blib/arch' : 'lib';
for my $block (@blocks) {
    my ($line, $code) = @$block;
    my ($out, $script) = tempfile(SUFFIX => '.pl', UNLINK => 1);
    binmode $out, ':encoding(UTF-8)';
    print $out "use v5.36;\nuse utf8;\n#line $line \"$guide\"\n$code";
    close $out;
    my $output = `"$^X" "-I$lib" "-I$arch" "$script" 2>&1`;
    is $?, 0, "example at line $line runs" or diag $output;
}

done_testing;
