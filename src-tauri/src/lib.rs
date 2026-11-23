pub mod extract;
pub mod index;

#[cfg(feature = "app")]
mod app;

#[cfg(feature = "app")]
pub use app::run;

// THROWAWAY: proves CI turns red; reverted in the next commit.
pub fn ci_break_clippy(v: &[u8]) -> bool {
    v.len() == 0
}

#[cfg(test)]
#[test]
fn ci_break_fails() {
    assert_eq!(1 + 1, 3);
}
