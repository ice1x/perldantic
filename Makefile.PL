use v5.36;
use ExtUtils::MakeMaker;
use FFI::Build::MM;

use lib 'inc';
use Perldantic::Prebuilt;

# The core library is built from its Rust sources (`source`), or fetched as the release built it
# for this platform (`prebuilt`): PERLDANTIC_BUILD chooses, and by default Rust is used when
# cargo runs. PERLDANTIC_PREBUILT_URL fetches from elsewhere (a file:// directory in CI).
my $mode = eval { Perldantic::Prebuilt::mode() } // do { warn $@; exit 0 };
my ($fbmm, $library);
if ($mode eq 'source') {
    $fbmm = FFI::Build::MM->new;
}
else {
    # without a library there is nothing to build: no Makefile, which CPAN clients report as
    # an unsupported platform rather than a failure
    $library = eval { Perldantic::Prebuilt::fetch(url => $ENV{PERLDANTIC_PREBUILT_URL}) } // do { warn $@; exit 0 };
    print "Using the prebuilt core library $library\n";
}

my %args = (
    NAME             => 'Perldantic',
    DISTNAME         => 'Perldantic',
    VERSION_FROM     => 'lib/Perldantic.pm',
    ABSTRACT_FROM    => 'lib/Perldantic.pm',
    AUTHOR           => 'ice1x',
    LICENSE          => 'mit',
    MIN_PERL_VERSION => '5.036',
    CONFIGURE_REQUIRES => {
        'ExtUtils::MakeMaker'       => '6.64',
        'FFI::Build::MM'            => '2.00',
        'FFI::Build::File::Cargo'   => '0.17',
    },
    PREREQ_PM => {
        'Class::Method::Modifiers'  => '2.00',
        'Cpanel::JSON::XS'          => '4.08',
        'FFI::Platypus'             => '2.00',
        'FFI::Platypus::Lang::Rust' => '0.17',
        'Role::Tiny'                => '2.002',
    },
    TEST_REQUIRES => {
        'Test2::V0'       => '0.000159',
        'Test::LeakTrace' => '0.17',
        'Type::Tiny'      => '1.012',
        'DateTime'        => '1.50',
        'Time::Moment'    => '0.44',
        'URI'             => '1.60',
    },
    META_MERGE => {
        'meta-spec' => { version => 2 },
        no_index    => {directory => [qw(tests crates ffi docs inc prebuilt)]},
        resources   => {
            bugtracker => {web => 'https://github.com/ice1x/perldantic/issues'},
            repository => {
                type => 'git',
                url  => 'https://github.com/ice1x/perldantic.git',
                web  => 'https://github.com/ice1x/perldantic',
            },
        },
    },
);
WriteMakefile($fbmm ? $fbmm->mm_args(%args) : %args);

sub MY::postamble {
    return $fbmm->mm_postamble if $fbmm;
    # put the prebuilt library where FFI::Build would have built it
    return <<"MAKE";
pure_all :: perldantic_prebuilt

perldantic_prebuilt :
\t\$(NOECHO) \$(ABSPERLRUN) -Iinc -MPerldantic::Prebuilt -e "Perldantic::Prebuilt::install_blib(q{$library})"
MAKE
}
