# tabmux fuzz targets

These targets are **non-blocking** and run on nightly with
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz). They are deliberately
outside the root package so `cargo test` and normal CI remain fast and stable.

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run csv
cargo +nightly fuzz run xlsx
cargo +nightly fuzz run html
```

Each target feeds arbitrary bytes to a reader. The contract is simple: the
reader may return an error, but it must not panic, hang, or read outside the
provided buffer. Crash artifacts are written to `fuzz/artifacts/` and should be
added to `fuzz/corpus/<target>/` as regression inputs when fixed.

Suggested starting corpora: the checked-in `fixtures/basic.csv`,
`fixtures/sales.xlsx`, and `fixtures/tables.html` files.
