# fvp-rs

A Rust library for packing and unpacking FVP game engine `.bin` archive.

compare to:

- [Leticiel/fvp-tools](https://github.com/Leticiel/fvp-tools) (Python): fvp-rs is faster and easy to use.
- [Nikaidou-Shinku/fvp-unpacker](https://github.com/Nikaidou-Shinku/fvp-unpacker) (Rust): fvp-rs provides packing ability.

## Usage

1. download the latest release from [releases](https://github.com/lxl66566/fvp-rs/releases), extract the fvp binary.
2. packing:
   ```shell
   fvp pack input_dir [output_file]
   ```
3. unpacking:
   ```shell
   fvp unpack input_file [output_dir]
   ```
