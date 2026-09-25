#!/usr/bin/env perl
# Write prebuilt/SHA256SUMS for the core libraries in a directory: the checksums Makefile.PL
# checks prebuilt downloads against, and the URL they are downloaded from.
#
#   perl tools/release/prebuilt-sums.pl <directory> <release download URL>
use v5.36;
use File::Path qw(make_path);
use lib 'inc';
use Perldantic::Prebuilt;

my ($dir, $url) = @ARGV;
die "usage: $0 <directory> <release download URL>\n" if !defined $url || !-d $dir;
opendir my $dh, $dir or die "$dir: $!\n";
my @files = sort grep { /\Alibperldantic-[\w-]+\.(?:so|dylib)\z/ } readdir $dh;
die "no libperldantic-<target> libraries in $dir\n" if !@files;
make_path('prebuilt');
open my $out, '>', $Perldantic::Prebuilt::SUMS or die "$Perldantic::Prebuilt::SUMS: $!\n";
print $out "# $url\n";
printf $out "%s  %s\n", Perldantic::Prebuilt::sha256_of("$dir/$_"), $_ for @files;
close $out or die "$Perldantic::Prebuilt::SUMS: $!\n";
print "wrote $Perldantic::Prebuilt::SUMS for @files\n";
