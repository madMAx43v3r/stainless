# Stainless Hello World

This is a complete runnable Stainless program. From the repository root:

```sh
cargo run -p stainlessc -- --run examples/hello-world/main.stl
```

With `stainlessc` installed, run `stainlessc --run main.stl` from this
directory. To keep an executable instead, run
`stainlessc --build -o hello main.stl`.

It prints:

```text
Hello, world!
```

The project has one source file: [`main.stl`](main.stl). Its root `i32 main()`
is the executable entry point. `stainlessc --run` transpiles it, generates the
small Rust entry point internally, invokes `rustc`, runs the result, and cleans
up its temporary build files.

There is deliberately no Cargo package, `build.rs`, or Rust `main.rs` here.
