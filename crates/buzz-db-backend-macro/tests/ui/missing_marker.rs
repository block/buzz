use buzz_db_backend_macro::enforce_sqlite_backend_declarations;

struct Db;

#[enforce_sqlite_backend_declarations]
impl Db {
    pub fn omitted(&self) {}
}

fn main() {}
