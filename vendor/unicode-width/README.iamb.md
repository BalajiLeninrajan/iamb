# unicode-width 0.2.2, patched for iamb

This is the crates.io release of `unicode-width` 0.2.2, unchanged except for
one edit in `src/tables.rs`: `lookup_width` and `lookup_width_cjk` return width 1 for U+0BBE (TAMIL VOWEL SIGN AA, ா) and
U+0BD7 (TAMIL AU LENGTH MARK, ௗ). Upstream gives them width 0 because they
are `Grapheme_Extend`.

Ghostty clusters graphemes (mode 2027) and draws a cluster such as நா in two
cells. With upstream's widths, ratatui reserves one, so every Tamil word with
ா leaves iamb a column behind the terminal, and its next cursor move overwrites
half of a cluster: letters vanish. Counting these signs as one column makes
the two agree. Other Tamil signs already matched.

`Cargo.toml` routes every `unicode-width 0.2` in the dependency tree here
through `[patch.crates-io]`, so ratatui and iamb both see the change.
