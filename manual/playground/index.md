# Playground

A browser playground is planned, but the reliable way to try AIVI today is locally from the repository.

## Checking and formatting files

```bash
cargo run -p aivi-cli -- check path/to/file.aivi
cargo run -p aivi-cli -- fmt path/to/file.aivi
```

## Running programs

Use `aivi run` when your file exports GTK markup as the application entry point.

Use `aivi execute` when your file exports a headless `Task` you want to run without a UI.

For grounded examples, start with `fixtures/`, `demos/`, and the reference in `aivi.md`.
