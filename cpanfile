requires 'perl', '5.036';
requires 'Class::Method::Modifiers', '2.00';
requires 'Cpanel::JSON::XS', '4.08';
requires 'FFI::Platypus', '2.00';
requires 'FFI::Platypus::Lang::Rust', '0.17';
requires 'Role::Tiny', '2.002';

on configure => sub {
    requires 'ExtUtils::MakeMaker', '6.64';
    requires 'FFI::Build::MM', '2.00';
    requires 'FFI::Build::File::Cargo', '0.17';
};

on test => sub {
    requires 'Test2::V0', '0.000159';
    requires 'Test::LeakTrace', '0.17';
    requires 'DateTime', '1.50';
    requires 'Time::Moment', '0.44';
    requires 'URI', '1.60';
};
