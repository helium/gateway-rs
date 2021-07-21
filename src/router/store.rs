use crate::{link_packet::Packet, Error, Region, Result};
use chrono::Utc;
use helium_proto::Message;
use rusqlite::{params, types::ToSqlOutput, Connection, OptionalExtension, ToSql};
use std::{fs, path::Path};

pub const VERSION: u16 = 0;

pub struct RouterStore {
    connection: rusqlite::Connection,
}

impl RouterStore {
    pub fn new(path: &Path) -> Result<Self> {
        let connection = rusqlite::Connection::open(path)?;
        let result = match get_version(&connection)? {
            None => {
                init_store(&connection)?;
                connection
            }
            Some(stored_version) if stored_version != VERSION => {
                drop(connection);
                fs::remove_file(path)?;
                let connection = rusqlite::Connection::open(path)?;
                init_store(&connection)?;
                connection
            }
            _ => connection,
        };
        Ok(Self { connection: result })
    }

    pub fn store_packet(&self, region: Region, packet: &Packet) -> Result {
        let mut stmt = self
            .connection
            .prepare_cached("INSERT INTO packets (received, region, packet)  values (?, ?, ?)")?;
        let _ = stmt.execute(params![Utc::now(), region, packet])?;
        Ok(())
    }
}

fn get_version(conn: &Connection) -> Result<Option<u16>> {
    conn.query_row(
        "SELECT value FROM metadata where name = 'version'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(Error::from)
}

fn update_metadata<V>(conn: &Connection, key: &str, value: V) -> rusqlite::Result<usize>
where
    V: ToSql,
{
    conn.execute(
        "REPLACE INTO metadata (name, value) VALUES(?, ?)",
        params![key, value],
    )
}

fn init_store(conn: &Connection) -> Result {
    init_metadata(conn)
        .and_then(|_| init_store_packets(conn))
        .and_then(|_| update_metadata(conn, "version", VERSION))
        .map(|_| ())
        .map_err(Error::from)
}

fn init_metadata(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS metadata(name TEXT PRIMARY KEY, value)",
        [],
    )
}

fn init_store_packets(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS packets (
            received TEXT,
            region TEXT, 
            packet BLOB
        )",
        [],
    )
}

impl ToSql for Region {
    fn to_sql(&self) -> std::result::Result<ToSqlOutput<'_>, rusqlite::Error> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl ToSql for Packet {
    fn to_sql(&self) -> std::result::Result<ToSqlOutput<'_>, rusqlite::Error> {
        let mut buf = vec![];
        self.encode(&mut buf)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(ToSqlOutput::from(buf))
    }
}
