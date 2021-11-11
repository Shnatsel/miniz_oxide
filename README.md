Main library [![Crates.io](https://img.shields.io/crates/v/miniz_oxide.svg)](https://crates.io/crates/miniz_oxide)[![Docs](https://docs.rs/miniz_oxide/badge.svg)](https://docs.rs/miniz_oxide)

C API [![Crates.io](https://img.shields.io/crates/v/miniz_oxide_c_api.svg)](https://crates.io/crates/miniz_oxide_c_api)[![Docs](https://docs.rs/miniz_oxide_c_api/badge.svg)](https://docs.rs/miniz_oxide_c_api)

# miniz_oxide
Pure rust Rust replacement for the [miniz](https://github.com/richgel999/miniz) deflate/zlib encoder/decoder using no unsafe code. Builds in [no_std](https://docs.rust-embedded.org/book/intro/no-std.html) mode, though requires the use of `alloc` and `collections` crates.

This project is organized into a C API shell and a rust crate.
The Rust crate is found in the [miniz_oxide subdirectory](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide).

miniz_oxide 0.5.x Requires at least rust 1.40.0, 0.3.x requires at least rust 0.36.0.

For a friendlier streaming API using readers and writers, [flate2](https://crates.io/crates/flate2) can be used, which can use miniz_oxide as a rust-only back-end.

## miniz_oxide_C_API
The C API is intented to replicate the api exported from miniz, and in turn also part of zlib. The C header is generated using [cbindgen](https://github.com/eqrion/cbindgen). The current implementation has not seen a lot of testing outside of automated test, is a bit weak in ddocumentation and should be seen as experimental.

The data structures do not share the exact same layout that is specified in miniz.h (from the original miniz), and should thus be allocated via the included functions.

### API documentation

TODO

### Testing

```bash
$ cargo test
$ ./test.sh
```

### Benches
```bash
$ cargo bench --features=benching
```
or to compare to miniz
```bash
$ ./travis-after-success.sh
```

### Including in C/C++ projects

Link against the `libminiz_oxide_c_api.a` generated by `build.sh`. The generated header that can be used `miniz.h` (using the original miniz headers may or may not work), which currently also uses `miniz_extra_defs.h` for some static definitions.

### Cargo-fuzz testing

Install fuzzer:
```bash
$ cargo install cargo-fuzz
```

Run fuzzer:
```bash
$ ./run_fuzz.sh
```

### License
This library (excluding the miniz C code used for tests) is licensed under the MIT license. The library is based on the miniz C library, of which the parts used are dual-licensed under the [MIT license](https://github.com/Frommi/miniz_oxide/blob/master/miniz/miniz.c#L1) and also the [unlicense](https://github.com/Frommi/miniz_oxide/blob/master/miniz/miniz.c#L577).
The parts of miniz that are not covered by the unlicense is [some Zip64 code](https://github.com/richgel999/miniz/commit/224d207ce8fffb908e156d27478be3afb5d83e6a#diff-edc0e9ccfae3b5324b85b3ec0a53dc74) which is only MIT licensed. This and other Zip functionality in miniz is not part of the miniz_oxidde and miniz_oxide_c_api rust libraries.
