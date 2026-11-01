# Pitfalls

Mistakes already made, and the rule for each. Add one in the same PR when you make a new one.

- **Rust from rustup in `~/.cargo/bin`** may not be on `PATH` in a fresh shell:
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Layout units**: the model is in millimetres from the band's corner; `Document` items are in
  millimetres from the page's corner (margins already added). Don't add the margin twice.
