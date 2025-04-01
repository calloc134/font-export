# Font Exporter

This project extracts font data from system fonts on Windows using Rust and the Windows API.

## Overview

The program:
- Reads configuration from a `config.toml` file that specifies the `font_name` and `output_filename`.
- Creates a compatible device context (DC) and a font using safe RAII wrappers in Rust.
- Extracts font data with the Windows `GetFontData` function.
- Writes the retrieved font data to a file.

## Setup and Usage

1. **Configuration:**  
   Edit `config.toml` with the desired font name and output file name.
   ```toml
   font_name = "Arial"
   output_filename = "Arial.ttf"
   ```

2. **Download the Release:**  
   Visit the [Releases](https://github.com/calloc134/font-export/releases) page and download the appropriate binary for your system.

3. **Run the Program:**  
   Extract the downloaded archive, edit the configuration in `config.toml` if needed, and execute
