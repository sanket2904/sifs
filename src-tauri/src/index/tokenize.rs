//! The contents index's own tokenizer, used outside the index: query words
//! and the words of a snippet are split and folded by the same code, with
//! the same arguments, as contents_fts splits and folds a file's text.

use anyhow::{bail, Result};
use rusqlite::{ffi, Connection};
use std::ffi::{c_char, c_int, c_void, CString};
use std::marker::PhantomData;
use std::ops::{ControlFlow, Range};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::ptr::null_mut;

/// contents_fts's tokenize option (migrate.rs), as a tokenizer's name and
/// arguments. tokenize_tests compares the two over every BMP character.
const CONTENTS: [&str; 7] =
    ["unicode61", "remove_diacritics", "2", "categories", "L* N* Co M*", "separators", "\u{fe0e}\u{fe0f}\u{20e3}"];

/// One tokenizer made by the connection's FTS5, deleted when dropped.
pub struct Tokenizer<'conn> {
    methods: ffi::fts5_tokenizer,
    made: *mut ffi::Fts5Tokenizer,
    conn: PhantomData<&'conn Connection>,
}

impl<'conn> Tokenizer<'conn> {
    /// contents_fts's tokenizer, through conn's FTS5.
    pub fn contents(conn: &'conn Connection) -> Result<Tokenizer<'conn>> {
        let api = fts5_api(conn)?;
        let name = CString::new(CONTENTS[0])?;
        let args: Vec<CString> = CONTENTS[1..].iter().map(|a| CString::new(*a)).collect::<Result<_, _>>()?;
        let mut argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
        // SAFETY: api is the live connection's FTS5 API; the strings outlive
        // the calls, which copy what they keep.
        unsafe {
            let mut user = null_mut();
            let mut methods: ffi::fts5_tokenizer = std::mem::zeroed();
            let Some(find) = (*api).xFindTokenizer else { bail!("FTS5 has no xFindTokenizer") };
            if find(api, name.as_ptr(), &mut user, &mut methods) != ffi::SQLITE_OK {
                bail!("FTS5 has no {} tokenizer", CONTENTS[0]);
            }
            let Some(create) = methods.xCreate else { bail!("the tokenizer has no xCreate") };
            let mut made = null_mut();
            if create(user, argv.as_mut_ptr(), argv.len() as c_int, &mut made) != ffi::SQLITE_OK || made.is_null() {
                bail!("could not make the contents tokenizer");
            }
            Ok(Tokenizer { methods, made, conn: PhantomData })
        }
    }

    /// Calls each with every word of text in order: the word as the index
    /// keeps it (possibly empty: a word of marks that fold to nothing), and
    /// where it is in text, in bytes. Stops when each breaks. Ok(false) if
    /// it did.
    pub fn words(&self, text: &str, mut each: impl FnMut(&str, Range<usize>) -> ControlFlow<()>) -> Result<bool> {
        let Ok(length) = c_int::try_from(text.len()) else { bail!("text too long to tokenize: {} bytes", text.len()) };
        let mut call = Call { each: &mut each, broke: false, panic: None, bad_word: false };
        let Some(tokenize) = self.methods.xTokenize else { bail!("the tokenizer has no xTokenize") };
        // SAFETY: made is alive until drop; call outlives the call, and word
        // is only called with it.
        let rc = unsafe {
            tokenize(
                self.made,
                (&mut call as *mut Call<'_>).cast::<c_void>(),
                ffi::FTS5_TOKENIZE_DOCUMENT,
                text.as_ptr().cast::<c_char>(),
                length,
                Some(word),
            )
        };
        if let Some(panic) = call.panic {
            resume_unwind(panic);
        }
        if call.bad_word {
            bail!("the tokenizer gave a word that is not UTF-8");
        }
        if rc != ffi::SQLITE_OK && rc != ffi::SQLITE_DONE {
            bail!("tokenizing failed: SQLite error {rc}");
        }
        Ok(!call.broke)
    }
}

impl Drop for Tokenizer<'_> {
    fn drop(&mut self) {
        if let Some(delete) = self.methods.xDelete {
            // SAFETY: made came from xCreate and is deleted once.
            unsafe { delete(self.made) };
        }
    }
}

type Each<'a> = dyn FnMut(&str, Range<usize>) -> ControlFlow<()> + 'a;

struct Call<'a> {
    each: &'a mut Each<'a>,
    broke: bool,
    panic: Option<Box<dyn std::any::Any + Send>>,
    bad_word: bool,
}

unsafe extern "C" fn word(
    ctx: *mut c_void,
    _flags: c_int,
    token: *const c_char,
    length: c_int,
    start: c_int,
    end: c_int,
) -> c_int {
    let call = &mut *ctx.cast::<Call<'_>>();
    let bytes = if length <= 0 || token.is_null() {
        &[][..]
    } else {
        std::slice::from_raw_parts(token.cast::<u8>(), length as usize)
    };
    let Ok(word) = std::str::from_utf8(bytes) else {
        call.bad_word = true;
        return ffi::SQLITE_ERROR;
    };
    let range = start as usize..end as usize;
    match catch_unwind(AssertUnwindSafe(|| (call.each)(word, range))) {
        Ok(ControlFlow::Continue(())) => ffi::SQLITE_OK,
        Ok(ControlFlow::Break(())) => {
            call.broke = true;
            ffi::SQLITE_DONE
        }
        Err(panic) => {
            call.panic = Some(panic);
            ffi::SQLITE_ERROR
        }
    }
}

/// The connection's FTS5 API, which SQLite hands out through `SELECT fts5(?)`.
fn fts5_api(conn: &Connection) -> Result<*mut ffi::fts5_api> {
    let mut api: *mut ffi::fts5_api = null_mut();
    // SAFETY: a statement on the live connection, finalized before return;
    // fts5() writes the API's address through the pointer bound.
    unsafe {
        let db = conn.handle();
        let mut stmt = null_mut();
        if ffi::sqlite3_prepare_v2(db, c"SELECT fts5(?1)".as_ptr(), -1, &mut stmt, null_mut()) != ffi::SQLITE_OK {
            bail!("SQLite has no FTS5");
        }
        ffi::sqlite3_bind_pointer(stmt, 1, (&mut api as *mut *mut ffi::fts5_api).cast(), c"fts5_api_ptr".as_ptr(), None);
        ffi::sqlite3_step(stmt);
        ffi::sqlite3_finalize(stmt);
    }
    if api.is_null() {
        bail!("SQLite gave no FTS5 API");
    }
    Ok(api)
}

#[cfg(test)]
#[path = "tokenize_tests.rs"]
mod tests;
