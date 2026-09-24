use v5.36;
use Test2::V0;

# Every module carries POD that is well formed, has the standard sections, documents each
# public method, and has a SYNOPSIS that runs as written.

use B ();
use File::Spec;
use File::Temp qw(tempfile);
use Pod::Checker;

my $lib = -d 'blib/lib' ? 'blib/lib' : 'lib';
my @files = sort grep { -f } ("$lib/Perldantic.pm", glob "$lib/Perldantic/*.pm");
ok scalar @files, 'found the modules';

my %HOOKS = map { $_ => 1 } qw(import DESTROY AUTOLOAD);

sub pod_of ($file) {
    open my $fh, '<', $file or die "$file: $!";
    my ($in, @pod) = (0);
    while (my $line = <$fh>) {
        $in = 1 if $line =~ /^=[a-z]/;
        push @pod, $line if $in;
        $in = 0 if $line =~ /^=cut/;
    }
    return join '', @pod;
}

sub section ($pod, $name) {
    return $pod =~ /^=head1 \Q$name\E\n(.*?)(?=^=head1 |^=cut|\z)/ms ? $1 : undef;
}

sub headings ($pod) {
    my @lines = $pod =~ /^=(?:head[2-4]|item)\s+(.*)$/mg;
    return join "\n", map { s/[A-Z]<+\s*(.*?)\s*>+/$1/gr } @lines;
}

sub public_subs ($file) {
    open my $fh, '<', $file or die "$file: $!";
    my @packages;
    while (my $line = <$fh>) {
        last if $line =~ /^__END__/;
        push @packages, $1 if $line =~ /^\s*package\s+([\w:]+)/;
    }
    my $path = File::Spec->rel2abs($file);
    my @subs;
    for my $package (@packages) {
        no strict 'refs';
        for my $name (sort keys %{"${package}::"}) {
            next if $name !~ /^[A-Za-z]\w*$/ || $HOOKS{$name};
            my $code = *{"${package}::$name"}{CODE} or next;
            my $cv = B::svref_2object($code);
            next if $cv->GV->STASH->NAME ne $package;
            next if File::Spec->rel2abs($cv->FILE) ne $path;
            push @subs, "${package}::$name";
        }
    }
    return @subs;
}

for my $file (@files) {
    my $module = $file =~ s{^\Q$lib\E/}{}r =~ s{/}{::}gr =~ s{\.pm$}{}r;
    subtest $module => sub {
        my $checker = Pod::Checker->new(-warnings => 1);
        open my $null, '>', File::Spec->devnull or die $!;
        $checker->parse_from_file($file, $null);
        is $checker->num_errors, 0, 'no POD errors';
        is $checker->num_warnings, 0, 'no POD warnings';

        my $pod = pod_of($file);
        like section($pod, 'NAME'), qr/^\Q$module\E - \S/m, 'NAME is "Module - abstract"';
        for my $name ('SYNOPSIS', 'DESCRIPTION', 'SEE ALSO') {
            ok defined section($pod, $name), "has $name";
        }

        require $file =~ s{^\Q$lib\E/}{}r;
        my $documented = headings($pod);
        my @missing = grep { my $name = s/.*:://r; $documented !~ /(?<![\w:])\Q$name\E\b/ } public_subs($file);
        is \@missing, [], 'every public sub is documented';

        my $synopsis = section($pod, 'SYNOPSIS') // '';
        my $code = join '', $synopsis =~ /^((?:[ \t]+\S.*)?\n)/mg;
        my ($fh, $script) = tempfile(SUFFIX => '.pl', UNLINK => 1);
        print $fh "use v5.36;\n#line 1 \"$module SYNOPSIS\"\n$code";
        close $fh;
        my $output = `"$^X" "-I$lib" "-I@{[ $lib =~ s{lib$}{arch}r ]}" "$script" 2>&1`;
        is $?, 0, 'SYNOPSIS runs' or diag $output;
    };
}

done_testing;
