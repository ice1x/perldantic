use v5.36;
use Test2::V0;

# The scripts in examples/ run and print what their comments say.

my %expected = (
    '01-models.pl'     => "43\nAnn\nopen\n2\nid: Input should be greater than 0\ncolour: Extra inputs are not permitted\n",
    '02-json.pl'       => qq(2026\n19.9\n{"number":"SO-17","placed":"2026-09-22","total":"19.90","lines":[{"qty":2,"sku":"A-1"}]}\n)
        . "2026-09-22\nnumber, placed, total\n",
    '03-validators.pl' => "ann\n********\nusername: Value error, must be alphanumeric\n"
        . "password: String should have at least 8 characters\n",
    '04-types.pl'      => "11\ninvalid\nHIGH\nababab\n",
    '05-moo.pl'        => "Address\n42\nbad zip rejected\n",
);

opendir my $dh, 'examples' or die "examples: $!";
my @scripts = sort grep {/\.pl\z/} readdir $dh;
is \@scripts, [sort keys %expected], 'every example is checked';

my $lib  = -d 'blib/lib'  ? 'blib/lib'  : 'lib';
my $arch = -d 'blib/arch' ? 'blib/arch' : 'lib';
for my $script (@scripts) {
    SKIP: {
        skip "$script needs Moo", 1 if $script =~ /moo/ && !eval { require Moo; 1 };
        my $output = `"$^X" "-I$lib" "-I$arch" examples/$script 2>&1`;
        is $output, $expected{$script}, $script;
    }
}

# the README's quick start, as written
subtest 'README quick start' => sub {
    open my $fh, '<:encoding(UTF-8)', 'README.md' or die "README.md: $!";
    my $readme = do { local $/; <$fh> };
    my ($code) = $readme =~ /^## Quick start\n\n```perl\n(.*?)^```$/ms or return fail 'no quick start';
    require File::Temp;
    my ($out, $path) = File::Temp::tempfile(SUFFIX => '.pl', UNLINK => 1);
    print $out "use v5.36;\n$code";
    close $out;
    my $output = `"$^X" "-I$lib" "-I$arch" "$path" 2>&1`;
    is $output, qq({"id":42,"title":"Login fails","status":"open","tags":[]}\n)
        . "id: Input should be greater than 0\ntitle: Field required\n";
};

done_testing;
