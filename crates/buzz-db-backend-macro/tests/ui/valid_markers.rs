use buzz_db_backend_macro::enforce_sqlite_backend_declarations;

struct Db;

#[enforce_sqlite_backend_declarations]
impl Db {
    #[sqlite_backend(implemented)]
    #[allow(dead_code)]
    pub fn implemented(&self) {}

    /// Deliberately unsupported operation.
    #[sqlite_backend(unsupported = "requires postgres-only advisory locks")]
    pub async fn unsupported(&self) {}

    fn private(&self) {}
}

fn main() {}
