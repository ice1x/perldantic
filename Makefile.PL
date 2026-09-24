use v5.36;
use ExtUtils::MakeMaker;
use FFI::Build::MM;

my $fbmm = FFI::Build::MM->new;

WriteMakefile(
    $fbmm->mm_args(
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
            resources   => {
                repository => {
                    type => 'git',
                    url  => 'https://github.com/ice1x/perldantic.git',
                    web  => 'https://github.com/ice1x/perldantic',
                },
            },
        },
    )
);

sub MY::postamble {
    $fbmm->mm_postamble;
}
