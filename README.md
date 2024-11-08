# mp42mcap

Convert MP4 video files to MCAP format with H.264/H.265 compression.

AI wrote this code so no guarantees.

## Usage

```sh
$ cargo run -- --help

Converts MP4 videos to MCAP

Usage: mp42mcap [OPTIONS] <INPUT> <OUTPUT>

Arguments:
  <INPUT>   Input MP4 file
  <OUTPUT>  Output MCAP file

Options:
      --topic <TOPIC>        Topic name for the video messages [default: video]
      --frame-id <FRAME_ID>  Frame ID for the video messages [default: video]
  -h, --help                 Print help
  -V, --version              Print version
```
