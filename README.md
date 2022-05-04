# BNM exchange rate exporter

The main use case of the tool is to scan records from the given CSV file,
assuming that there's a column with valid date, fetch BNM's exchange rate
for that date and add it as a new column.

There are a few options that allow to:
- change format of the in/out date;
- change name and position of the exchange rate column;
- filter records using regexp;
- process a file with/without headers;
- process a file with custom CSV delimiter;

Run `./bnm-exporter -h` to see all available options.

**Note:** In case an error is occurred while processing a specific record (e.g. invalid date format),
that record is skipped with a warning message (set `RUST_LOG=warn` env variable for custom log level).

## Usage

There are Linux/GNU and MacOS binaries attached to the latest [release](https://github.com/Alexx-G/bnm-exporter/releases/latest).

**Note:** MacOS users may have to explicitly add the executable to the allow-list (given it's not signed and notarized), when opening it the first time.
Check Apple's [support article](https://support.apple.com/en-us/HT202491)
(the "If you want to open an app that hasn’t been notarized or is from an unidentified developer" section).


### Examples

Processes a CSV file with default date format and delimiter, and prints the result to STDOUT.
The CSV file is expected to have a "DATE" column, exchange rate is appended as the last column
(with default column name).

```bash
./bnm-exporter -i file.csv -d DATE
```

Same as previous one, but applies filtering (regex search) to the "DESCRIPTION" column and saves output to the specified file.

```bash
./bnm-exporter -i file.csv -d DATE -f "DESCRIPTION=foo|bar[0-9]" -o out.csv
```

Changes date format of the resulting CSV file.

```bash
./bnm-exporter -i file.csv -d DATE --out-date-format "%d.%m.%Y"
```

Changes column name and position (inserted after "AMOUNT" column) of the exchange rate column.

```bash
./bnm-exporter -i file.csv -d DATE --out-exchange-column EXCHANGE --out-exchange-insert-after AMOUNT
```

## Building

Get [Rust](https://rustup.rs/), install the stable [stable channel](https://rust-lang.github.io/rustup/concepts/channels.html)
and just build the project using [Cargo](https://doc.rust-lang.org/cargo/) (`cargo build`).
