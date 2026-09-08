# Starship fixtures

Ground-truth templates and expected outputs used to pin the template engine's
behavior against the real `starship` binary.

`default_modules.toml` holds the verbatim default `format` strings for the modules
GPY mirrors. To regenerate expected outputs, run the captured templates through the
installed `starship` (see `template_golden_tests.rs`).
