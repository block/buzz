use buzz_db_backend_macro::enforce_sqlite_backend_declarations;
struct Db;
#[enforce_sqlite_backend_declarations]
impl Db {
    #[sqlite_backend(sometimes)]
    pub fn malformed(&self) {}
}
fn main() {}
