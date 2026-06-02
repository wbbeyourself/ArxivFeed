mod config;
mod utils;
mod core;
mod v1;

use anyhow::Result;
use chrono::{Duration, Utc};
use tracing::{info, span};

use crate::config::{Config, Version};
use crate::core::{dump_cache, fetch_arxivs, from_cache};
use crate::core::{ArxivCollection, ArxivQueryBuilder};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .try_init()
        .expect("Tracing init error!");
    let root = span!(tracing::Level::INFO, "<FEED>");
    let _enter = root.enter();

    let config = Config::new()?;
    let client = reqwest::Client::builder().build()?;

    let today = Utc::now();
    let cache_day = today - Duration::days(std::cmp::max(config.limit_days, 1));

    // Re-group cached data by date (merge entries with different timestamps on the same day)
    let mut raw_data: ArxivCollection = from_cache(&config.cache_url, &client).await;
    raw_data = raw_data.into_iter().fold(
        ArxivCollection::new(),
        |mut acc: ArxivCollection, (date, categories)| {
            let truncated = date.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
            let entry = acc.entry(truncated).or_default();
            for (title, papers) in categories {
                let sub = entry.entry(title).or_default();
                sub.extend(papers);
            }
            acc
        },
    );
    for source in &config.sources {
        info!("Get: {}", source.category);
        let query = ArxivQueryBuilder::new()
            .search_query(&format!("cat:{}", source.category))
            .start(0)
            .max_results(source.limit)
            .sort_by("lastUpdatedDate") // "lastUpdatedDate" | "submittedDate"
            .sort_order("descending")
            .build();
        let arxivs = fetch_arxivs(query, &client).await?;
        for arxiv in arxivs {
            // Truncate to date-only to group papers by day, not by exact timestamp
            let date = arxiv.updated.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
            if date >= cache_day {
                let entry = raw_data.entry(date).or_default();
                let entry = entry.entry(String::from(&source.title)).or_default();
                entry.insert(arxiv);
            }
        }
        // Respect Arxiv API rate limit: wait 3 seconds between requests
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    let raw_data = raw_data
        .into_iter()
        .filter(|(d, _)| d >= &cache_day)
        .collect();

    dump_cache(&raw_data, &config)?;

    match config.version {
        Version::V1 => {
            v1::main(&config, raw_data)?;
        }
        Version::V2 => {
            todo!()
        }
    }

    Ok(())
}
