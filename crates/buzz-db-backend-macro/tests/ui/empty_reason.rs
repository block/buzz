use buzz_db_backend_macro::enforce_sqlite_backend_declarations;
struct Db;
#[enforce_sqlite_backend_declarations]
impl Db {
    #[sqlite_backend(unsupported = "  ")]
    pub fn empty(&self) {}
}
fn main() {}
