<div align="center">

# jive

**A minimialistic music player**

[![CI](https://github.com/cazeth/jive/actions/workflows/ci.yml/badge.svg)](https://github.com/cazeth/jive/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/jive.svg)](https://github.com/cazeth/jive/blob/main/LICENSE)

</div>

Jive is an ultra-simple music player ideal for shuffling music from a directory. It uses a music selection algorithm that avoids repetitiveness while prioritizing the music you seem to enjoy. I use it as my lofi music player.

## Install

Install [mpv](https://mpv.io), then:

```console
cargo install jive --locked
```

Building jive needs Rust 1.88 or later.

## Use

point jive to a music directory with `--root`.

```console
jive --root ~/music    # First run, the music directory perists.
jive                   # Then you can just start it.
```

Press `n` for another track

Run `jive-debug` to see the numbers the shuffle draws from: the preference,
staleness, and reliability of each track, and the share of the next draw they
add up to.

## License

[MIT](https://github.com/cazeth/jive/blob/main/LICENSE)
