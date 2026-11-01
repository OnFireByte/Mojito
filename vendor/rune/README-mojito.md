# Vendored rune

This is `anomalyco/rune` branch `dev` at rev `092dec2e40436d6835fe25ebc8a994fe67f3c9e4`, pruned to the crates Mojito builds, with one local change:

- `crates/rune/src/text/atlas.rs`: `ATLAS_SIZE` 2048 -> 512. A glyph page is a 1-byte-per-pixel mask kept both in the heap and as a GPU texture; at 2048 three idle renderers held 12 MB of mostly-blank atlas. 512 pages cost 256 KB each and glyphs that do not fit spill to a fresh page.
- `crates/rune/src/pixels.rs`: `IMAGE_ATLAS_SIZE` 1024 -> 512. An image page is RGBA; the switcher's app icons fit comfortably at 512 (a 512x512 page packs 256 32x32 icons).

The root `Cargo.toml` redirects the git dependencies here with `[patch.'https://github.com/anomalyco/rune']`.

To move to a newer rune: diff this tree against the upstream rev, re-apply the two const changes, and update every rev pin. Check `CHANGELOG.md` section `## dev` upstream for breaking changes first.
