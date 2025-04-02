# Font Exporter

This project extracts font data from system fonts on Windows using Rust and the Windows API.

## Overview

The program:

- Reads configuration from a `config.toml` file that specifies the `font_name` and `output_filename`.
- Creates a compatible device context (DC) and a font using safe RAII wrappers in Rust.
- Extracts font data with the Windows `GetFontData` function.
- Writes the retrieved font data to a file.

## Setup and Usage

1.  **Download the Release:**
    Visit the [Releases](https://github.com/calloc134/font-export/releases) page and download the appropriate binary (`font-export.exe`) for your Windows system.

2.  **Run the Program:**
    Open a command prompt or PowerShell in the directory where you saved the downloaded executable. Execute the program using command-line arguments, primarily using the short options:

    - **`-f <FONT_NAME>` (Required):** Specify the name of the font installed on your system that you want to extract (e.g., "Arial", "Times New Roman", "Meiryo UI"). **This argument is mandatory.** (Long form: `--font-name`)
    - **`-o <DIRECTORY_PATH>` (Optional):** Specify the directory where the extracted font file should be saved. The output file name will be the same as the specified `<FONT_NAME>`. **If omitted, the font file will be saved in the current directory (`.`).** (Long form: `--output-dir`)

    **Examples:**

    - **Extract "Arial" font to the current directory (using mandatory `-f`):**

      ```bash
      .\font-export.exe -f "Arial"
      ```

      (This will create a file named `Arial` in the current directory.)

    - **Extract "Times New Roman" font to a specific directory (e.g., `C:\MyFonts`) using short options:**

      ```bash
      .\font-export.exe -f "Times New Roman" -o "C:\MyFonts"
      ```

      (This will create the directory `C:\MyFonts` if it doesn't exist, and save the font as `C:\MyFonts\Times New Roman`.)

    - **Extract "Meiryo UI" font to a subdirectory named `output` relative to the current location:**
      ```bash
      .\font-export.exe -f "Meiryo UI" -o .\output
      ```
      (This will create `./output/Meiryo UI`.)

3.  **Get Help:**
    To see all available options (including both short and long forms) and their descriptions, run the program with the `-h` or `--help` flag:
    ```bash
    .\font-export.exe -h
    ```
