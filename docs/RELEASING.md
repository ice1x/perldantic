# Releasing

The CPAN distribution is built with ExtUtils::MakeMaker. It ships the Rust sources that the
build compiles (`crates/perldantic-core`, `ffi`, `Cargo.toml`, `Cargo.lock`), the XS encoder,
the Perl modules, the tests and the Perl conformance data (`tests/conformance/upstream`).
`MANIFEST.SKIP` leaves out the upstream snapshot, the recording tools and the Rust-only test
data. Installing builds the core with `cargo` (network access for crates.io) when Rust is
installed, and otherwise fetches the core library prebuilt for the platform from the GitHub
release named in `prebuilt/SHA256SUMS`, checked against the checksums there
(`inc/Perldantic/Prebuilt.pm`).

1. Build the prebuilt core libraries: run the *Prebuilt* workflow by hand
   (`gh workflow run prebuilt.yml -f tag=v0.2.0`) on the commit to release. It builds the
   libraries, installs the distribution from them without Rust, and drafts the release `v0.2.0`
   with the libraries and their `SHA256SUMS`. Put that file in the distribution and commit it
   with the release changes (the libraries stay valid as long as `crates/` and `ffi/` do not
   change):

   ```sh
   gh release download v0.2.0 --pattern SHA256SUMS --dir prebuilt --clobber
   ```

2. Set the version in every `lib/**/*.pm` (`our $VERSION`) and the workspace `Cargo.toml`, and
   date the release in `Changes`.
3. Regenerate `MANIFEST` from the tracked files (`make manifest` trips over the symlinks of
   `tools/.venv`):

   ```sh
   git ls-files | perl -MExtUtils::Manifest=maniskip -e \
     'my $skip = maniskip(); print grep { chomp; !$skip->($_) and $_ .= "\n" } <STDIN>' > MANIFEST
   echo MANIFEST >> MANIFEST && sort -u -o MANIFEST MANIFEST
   ```

4. Build and test the distribution from its own directory:

   ```sh
   perl Makefile.PL && make disttest && make dist
   ```

5. Merge the release PR, then tag the merge commit on `main` (`git tag v0.1.0` for
   `Perldantic-0.01`) and push the tag. Publish the draft release of the same name.
6. Upload `Perldantic-<version>.tar.gz` to PAUSE (https://pause.perl.org, or `cpan-upload`).
