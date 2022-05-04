use std::collections::HashMap;

use chrono::NaiveDate;
use clap::Parser;
use csv::{Reader, StringRecord, Writer, WriterBuilder};
use eyre::{eyre, Result};
use futures::future::join_all;
use lazy_static::lazy_static;
use regex::Regex;
use reqwest::StatusCode;

lazy_static! {
    static ref CURRENCY_CACHE: tokio::sync::RwLock<HashMap<String, f64>> =
        tokio::sync::RwLock::new(HashMap::new());
}

#[derive(Debug, Parser)]
/// CLI helper which parses a CSV file and adds BNM exchange rates for corresponding date.
struct OptionsParser {
    #[clap(long = "in-file", short = 'i', parse(from_os_str))]
    /// Path to the input file in CSV format.
    /// By default the file is expected to have headers as the first row.
    in_file: std::path::PathBuf,

    #[clap(long = "in-no-headers")]
    /// Must be set, in case the CSV file has no headers.
    /// In case CSV file has no headers, all options that allow specifying a column
    /// are interpreted as indexes (starting from 0).
    in_no_headers: bool,

    #[clap(long = "in-date-format", default_value = "%m/%d/%Y")]
    /// Date format of the input CSV file.
    in_date_format: String,

    #[clap(long = "in-column-delimiter", default_value = ",")]
    /// Column delimiter of the input CSV file.
    in_column_delimiter: char,

    #[clap(long = "in-date-column", short = 'd')]
    /// In case the input CSV file has header, it's used as header name.
    /// Otherwise it's used as an index (starting from 0).
    in_date_column: String,

    #[clap(long = "out-file", short = 'o')]
    /// Path to the output CSV file. If  omitted will be printed to STDOUT
    out_file: Option<std::path::PathBuf>,

    #[clap(long = "out-column-delimiter")]
    // Column delimiter of the output CSV file.
    out_column_delimiter: Option<char>,

    #[clap(long = "out-date-format")]
    /// Date format of the output file.
    /// If not provided, same format as input date will be used.
    out_date_format: Option<String>,

    #[clap(long = "out-exchange-column", default_value = "Exchange Rate")]
    /// Column name of the exchange rate.
    out_exchange_column: String,

    #[clap(long = "out-exchange-insert-after")]
    /// The column name/index exchange rate must be appended after.
    /// In case the input CSV file has header, it's used as header name.
    /// Otherwise it's used as an index.
    /// If not provided, it'll be appended as the last column.
    out_exchange_insert_after: Option<String>,

    #[clap(long = "filter", short = 'f')]
    /// The filter expression must be in {column}={regex} format.
    /// In case the input CSV file has header, {column} is used as header name.
    /// Otherwise it's used as an index.
    filter: Option<String>,
}

struct RecordFilter {
    column: usize,
    regex: Regex,
}

impl RecordFilter {
    fn matches(&self, record: &StringRecord) -> bool {
        record
            .get(self.column)
            .map(|v| self.regex.find(v).is_some())
            .unwrap_or(false)
    }
}

async fn fetch_exchange_rate(date: &NaiveDate) -> Result<f64> {
    let formatted_date = date.format("%d.%m.%Y").to_string();
    if CURRENCY_CACHE.read().await.contains_key(&formatted_date) {
        return CURRENCY_CACHE
            .read()
            .await
            .get(&formatted_date)
            .ok_or(eyre!("Failed to read from cache"))
            .map(|f| *f);
    }
    let url = format!("https://www.bnm.md/ro/export-official-exchange-rates?date={formatted_date}");
    log::debug!("Fetching exchange from {}", &url);
    let response = reqwest::get(&url).await?;
    if response.status() != StatusCode::OK {
        return Err(eyre!("Got unexpected status - {}", response.status()));
    }
    let body = response.text().await?;
    for line in body.lines().skip(2) {
        if line.contains(";USD;") {
            let rate: f64 = line.split(';').last().unwrap().replace(',', ".").parse()?;
            return Ok(rate);
        }
    }
    Err(eyre!("Didn't find required currency"))
}

fn get_column_index(headers: Option<&StringRecord>, column: &str) -> Result<usize> {
    match headers {
        Some(h) => h
            .iter()
            .position(|h| h == column)
            .ok_or_else(|| eyre!("Cannot find column \"{}\" in headers", column)),
        None => column
            .parse::<usize>()
            .map_err(|_| eyre!("Failed to parse column index - {}", column)),
    }
}

async fn add_exchange(
    date_column: usize,
    date_format: &str,
    out_date_format: Option<&String>,
    exchange_index: Option<usize>,
    record: StringRecord,
) -> Result<StringRecord> {
    let original_date = record
        .get(date_column)
        .ok_or_else(|| eyre!("Failed to lookup column {}", date_column))?;
    let date = NaiveDate::parse_from_str(original_date, date_format)?;
    let out_date = match out_date_format {
        Some(f) => date.format(f).to_string(),
        None => original_date.to_string(),
    };
    let exchange_rate = fetch_exchange_rate(&date).await?;
    let mut record: Vec<String> = record.iter().map(|v| v.to_string()).collect();
    record[date_column] = out_date;
    match exchange_index {
        Some(v) => record.insert(v + 1, exchange_rate.to_string()),
        None => record.push(exchange_rate.to_string()),
    };
    Ok(StringRecord::from(record))
}

fn create_filter(filter: &str, headers: Option<&StringRecord>) -> Result<RecordFilter> {
    let (column, re) = filter
        .split_once('=')
        .ok_or(eyre!("The filter must be k=v pair"))?;
    let regex = Regex::new(re)?;
    let column = get_column_index(headers, column)?;
    Ok(RecordFilter { regex, column })
}

fn get_out_headers(
    headers: &StringRecord,
    exchange_column: &str,
    exchange_column_insert_after: Option<&String>,
) -> StringRecord {
    let exchange_column_index = exchange_column_insert_after.and_then(|v| {
        let index = get_column_index(Some(headers), v);
        index
            .map_err(|e| {
                log::warn!("Failed to get exchange column index - {}", e);
                e
            })
            .ok()
    });
    let mut record: Vec<String> = headers.iter().map(|v| v.to_string()).collect();
    match exchange_column_index {
        Some(v) => record.insert(v, exchange_column.to_string()),
        None => record.push(exchange_column.to_string()),
    };
    StringRecord::from(record)
}

fn read_records<T>(reader: &'_ mut Reader<T>, filter: Option<&RecordFilter>) -> Vec<StringRecord>
where
    T: std::io::Read,
{
    reader
        .records()
        .filter_map(|r| {
            r.map_err(|e| {
                log::warn!("Skipping row due to parse error - {}", e);
                e
            })
            .ok()
        })
        .filter(|r| match filter {
            Some(f) => f.matches(r),
            None => true,
        })
        .collect()
}

fn write_records<T>(
    records: &[StringRecord],
    headers: Option<StringRecord>,
    writer: &mut Writer<T>,
) -> Result<()>
where
    T: std::io::Write,
{
    if headers.is_some() {
        writer.write_record(&headers.unwrap())?;
    };
    for record in records {
        writer.write_record(record)?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    env_logger::init();
    let args = OptionsParser::parse();
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .delimiter(args.in_column_delimiter as u8)
        .has_headers(!args.in_no_headers)
        .from_path(&args.in_file)?;
    let headers = if reader.has_headers() {
        Some(reader.headers()?.clone())
    } else {
        None
    };
    let date_format = args.in_date_format.as_str();
    let out_date_format = args.out_date_format.as_ref();
    let date_index = get_column_index(headers.as_ref(), &args.in_date_column)?;
    let exchange_index = args
        .out_exchange_insert_after
        .as_ref()
        .and_then(|v| get_column_index(headers.as_ref(), v).ok());
    let filter = args
        .filter
        .as_ref()
        .and_then(|f| create_filter(f, headers.as_ref()).ok());
    let futures = read_records(&mut reader, filter.as_ref())
        .into_iter()
        .map(|r| async move {
            add_exchange(date_index, date_format, out_date_format, exchange_index, r).await
        });
    let records = join_all(futures).await;
    let out_records: Vec<StringRecord> = records
        .into_iter()
        .filter_map(|r| {
            r.map_err(|e| {
                log::warn!("Failed to add exchange rate - {}", e);
                e
            })
            .ok()
        })
        .collect();
    let out_headers = headers.as_ref().map(|h| {
        get_out_headers(
            h,
            &args.out_exchange_column,
            args.out_exchange_insert_after.as_ref(),
        )
    });
    let out_delimiter = args
        .out_column_delimiter
        .unwrap_or(args.in_column_delimiter);
    let mut writer_builder = WriterBuilder::new();
    writer_builder
        .delimiter(out_delimiter as u8)
        .has_headers(out_headers.is_some());
    match args.out_file {
        None => {
            let mut writer = writer_builder.from_writer(std::io::stdout());
            write_records(&out_records, out_headers, &mut writer)?;
        }
        Some(v) => {
            let mut writer = writer_builder.from_path(v)?;
            write_records(&out_records, out_headers, &mut writer)?;
        }
    };
    Ok(())
}
