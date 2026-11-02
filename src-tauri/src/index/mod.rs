//! The file index: crawl, watch and search the folders the user chose.

pub mod crawl;
pub mod db;
pub mod ignore;
pub mod search;
pub mod watch;

#[cfg(test)]
mod tests;
