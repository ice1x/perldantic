# Releasing

The CPAN distribution is built with ExtUtils::MakeMaker. It ships the Rust sources that the
build compiles (`crates/perldantic-core`, `ffi`, `Cargo.toml`, `Cargo.lock`), the XS encoder,
the Perl modules, the tests and the Perl conformance data (`tests/conformance/upstream`).
`MANIFEST.SKIP` leaves out the upstream snapshot, the recording tools and the Rust-only test
data. Installing needs a Rust toolchain (`cargo`) and network access for crates.io.

1. Set the version in every `lib/**/*.pm` (`our $VERSION`) and the workspace `Cargo.toml`, and
   date the release in `Changes`.
2. Regenerate `MANIFEST` from the tracked files (`make manifest` trips over the symlinks of
   `tools/.venv`):

   ```sh
   git ls-files | perl -MExtUtils::Manifest=maniskip -e \
     'my $skip = maniskip(); print grep { chomp; !$skip->($_) and $_ .= "\n" } <STDIN>' > MANIFEST
   echo MANIFEST >> MANIFEST && sort -u -o MANIFEST MANIFEST
   ```

3. Build and test the distribution from its own directory:

   ```sh
   perl Makefile.PL && make disttest && make dist
   ```

4. Merge the release PR, then tag the merge commit on `main` (`git tag v0.1.0` for
   `Perldantic-0.01`) and push the tag.
5. Upload `Perldantic-<version>.tar.gz` to PAUSE (https://pause.perl.org, or `cpan-upload`).
