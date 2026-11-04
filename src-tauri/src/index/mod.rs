//! The file index: crawl, watch and search the folders the user chose.

pub mod crawl;
pub mod db;
pub mod folders;
pub mod ignore;
pub mod migrate;
pub mod query;
pub mod search;
pub mod watch;

#[cfg(test)]
pub(crate) mod tests;
