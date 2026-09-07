//! The durable half: V3 rows into a Lance dataset (feature `lance`).
//!
//! # The schema is derived, not written down
//!
//! Lance's columns come from [`NODE_ROW_COLUMNS`] — the canon's own
//! [`ColumnDescriptor`] table, the same one
//! [`SoaEnvelope::verify_layout`] validates the packet against. Three
//! columns, `key(16) | edges(16) | value(480)`, each a `FixedSizeBinary`
//! of exactly the width the descriptor states.
//!
//! Writing the widths out by hand would make this file a second source of
//! truth for a layout the contract already owns, and the failure would be
//! silent: rows would land, read back, and be wrong only for a consumer
//! that resolved them through the canon. So the schema is BUILT from the
//! descriptors, and [`schema`] is what a caller inspects. A layout change
//! upstream moves this schema with it or fails to compile.
//!
//! This is a partition of the row, never a copy of it: the three columns
//! are disjoint byte ranges that reassemble to the 512-byte row, which is
//! why there is no separate "key" column duplicating bytes that already
//! live at `0..16`.
//!
//! # What this is not
//!
//! Not a query engine, not an index, not a graph, and not `lancedb` — the
//! wanted operation is `Dataset::write` of a fixed schema, so this takes
//! `lance` directly (see the manifest for the second reason: lancedb
//! 0.38.0 does not compile without its `remote` feature). `lance-graph`'s planner
//! and the rest of the engine stay out of this workspace (the BBB rule);
//! what crosses is `lance-graph-contract`, which compiles types and has
//! zero dependencies of its own. A consumer that wants to QUERY these rows
//! runs lance-graph against the dataset — it does not link it here.

use std::sync::Arc;

use arrow_array::{Array, FixedSizeBinaryArray, RecordBatch, RecordBatchIterator};
use arrow_schema::{DataType, Field, Schema};
use lance::Dataset;
use lance::dataset::{WriteMode, WriteParams};
use lance_graph_contract::canonical_node::{NODE_ROW_COLUMNS, NODE_ROW_STRIDE, NodeRowColumn};
use lance_graph_contract::soa_envelope::{ColumnDescriptor, SoaEnvelope};

use crate::ProgramRows;

/// Why a Lance operation failed.
#[derive(Debug)]
pub enum LanceError {
    /// The dataset could not be opened, created or written.
    Store(String),
    /// Arrow rejected the batch — a schema or width mismatch.
    Arrow(String),
    /// The packet's own geometry check failed before anything was written.
    /// A bad envelope must never reach storage; this is the gate.
    Layout(String),
}

impl core::fmt::Display for LanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Store(m) => write!(f, "lance store: {m}"),
            Self::Arrow(m) => write!(f, "arrow: {m}"),
            Self::Layout(m) => write!(f, "envelope layout: {m}"),
        }
    }
}

impl std::error::Error for LanceError {}

/// The column name a descriptor's `name_id` stands for.
///
/// The ordinals are the contract's [`NodeRowColumn`]; the strings are this
/// crate's, because a Lance schema needs names and the canon deliberately
/// carries identity as an ordinal rather than a label (labels come from the
/// ClassView — the slot-purity rule).
const fn column_name(name_id: u16) -> &'static str {
    if name_id == NodeRowColumn::Key as u16 {
        "key"
    } else if name_id == NodeRowColumn::Edges as u16 {
        "edges"
    } else {
        "value"
    }
}

/// The Arrow schema for a table of V3 rows, derived from the canon's
/// column descriptors.
#[must_use]
pub fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(
        NODE_ROW_COLUMNS
            .iter()
            .map(|c| {
                let width = i32::try_from(c.col_bytes_per_row()).unwrap_or(i32::MAX);
                Field::new(
                    column_name(c.name_id),
                    DataType::FixedSizeBinary(width),
                    false,
                )
            })
            .collect::<Vec<_>>(),
    ))
}

/// Slice one column out of every row of `packet`'s backing bytes.
///
/// Reads through the descriptor's own byte range, so a column can only ever
/// be the bytes the canon says it is.
fn column_array(
    bytes: &[u8],
    rows: usize,
    c: &ColumnDescriptor,
) -> Result<FixedSizeBinaryArray, LanceError> {
    let (start, end) = c.row_byte_range();
    let width = end - start;
    let mut flat = Vec::with_capacity(rows * width);
    for r in 0..rows {
        let base = r * NODE_ROW_STRIDE;
        flat.extend_from_slice(&bytes[base + start..base + end]);
    }
    let width = i32::try_from(width).map_err(|e| LanceError::Arrow(e.to_string()))?;
    FixedSizeBinaryArray::try_new(width, flat.into(), None)
        .map_err(|e| LanceError::Arrow(e.to_string()))
}

/// One [`ProgramRows`] as an Arrow batch in the canon's column layout.
///
/// # Errors
///
/// [`LanceError::Layout`] if the packet's geometry does not validate —
/// checked BEFORE any bytes are shaped, so a malformed envelope cannot
/// reach a dataset; [`LanceError::Arrow`] if Arrow rejects a column.
pub fn batch_of(rows: &ProgramRows, cycle: u32) -> Result<RecordBatch, LanceError> {
    let packet = rows.packet(cycle);
    packet
        .verify_layout()
        .map_err(|e| LanceError::Layout(format!("{e:?}")))?;
    let bytes = packet.as_le_bytes();
    let n = packet.n_rows();
    let columns = NODE_ROW_COLUMNS
        .iter()
        .map(|c| column_array(bytes, n, c).map(|a| Arc::new(a) as Arc<dyn Array>))
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(schema(), columns).map_err(|e| LanceError::Arrow(e.to_string()))
}

/// A Lance dataset of V3 rows.
///
/// Holds its own runtime so the surface stays synchronous: this crate's
/// callers are the intake path and a bake binary, neither of which is
/// async, and handing them a runtime is cheaper than colouring the whole
/// workspace.
pub struct LanceStore {
    uri: String,
    rt: tokio::runtime::Runtime,
}

impl std::fmt::Debug for LanceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanceStore")
            .field("uri", &self.uri)
            .finish_non_exhaustive()
    }
}

impl LanceStore {
    /// A store over the dataset at `uri`. The dataset need not exist yet —
    /// the first [`append`](Self::append) creates it.
    ///
    /// # Errors
    ///
    /// [`LanceError::Store`] if the runtime cannot be built.
    pub fn open(uri: &str) -> Result<Self, LanceError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| LanceError::Store(e.to_string()))?;
        Ok(Self {
            uri: uri.to_string(),
            rt,
        })
    }

    /// Append `rows` to the dataset, creating it on the first write.
    ///
    /// `WriteMode::Append` against a dataset that does not exist yet is
    /// what Lance itself turns into a create, so there is no
    /// exists-then-write race to lose here.
    ///
    /// # Errors
    ///
    /// [`LanceError::Layout`] / [`LanceError::Arrow`] from [`batch_of`];
    /// [`LanceError::Store`] if Lance refuses the write.
    pub fn append(&self, rows: &ProgramRows, cycle: u32) -> Result<(), LanceError> {
        let batch = batch_of(rows, cycle)?;
        let schema = batch.schema();
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let params = WriteParams {
            mode: WriteMode::Append,
            ..Default::default()
        };
        self.rt
            .block_on(Dataset::write(reader, self.uri.as_str(), Some(params)))
            .map(|_| ())
            .map_err(|e| LanceError::Store(e.to_string()))
    }

    /// How many rows the dataset holds.
    ///
    /// # Errors
    ///
    /// [`LanceError::Store`] if the dataset cannot be opened or counted.
    pub fn count(&self) -> Result<usize, LanceError> {
        self.rt.block_on(async {
            let ds = Dataset::open(self.uri.as_str())
                .await
                .map_err(|e| LanceError::Store(e.to_string()))?;
            ds.count_rows(None)
                .await
                .map_err(|e| LanceError::Store(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CLASSID;
    use ogar_loco::LaneShape;

    fn rows() -> ProgramRows {
        let json = blockly_shim::templates::ALL[0].1;
        let scripts = blockly_shim::from_workspace_json(json).expect("parses");
        let prog = blockly_shim::templates::cast(LaneShape::Pairs, &scripts[0]).expect("casts");
        ProgramRows::from_program(&prog, CLASSID).expect("lays out")
    }

    /// The schema is the canon's, width for width — and the widths sum to
    /// the locked stride, so a column cannot quietly go missing.
    #[test]
    fn the_schema_is_derived_from_the_canon_column_table() {
        let s = schema();
        assert_eq!(s.fields().len(), NODE_ROW_COLUMNS.len());
        let mut total = 0usize;
        for (f, c) in s.fields().iter().zip(NODE_ROW_COLUMNS) {
            let want = i32::try_from(c.col_bytes_per_row()).expect("bounded");
            assert_eq!(
                f.data_type(),
                &DataType::FixedSizeBinary(want),
                "{}",
                f.name()
            );
            total += c.col_bytes_per_row();
        }
        assert_eq!(total, NODE_ROW_STRIDE, "the columns must partition the row");
    }

    /// A batch's three columns reassemble, row by row, into exactly the
    /// bytes the envelope holds — the partition is lossless and in order.
    ///
    /// Anti-vacuity: the row bytes are asserted non-zero first, so an
    /// implementation writing empty columns cannot pass by matching zeros.
    #[test]
    fn the_batch_columns_reassemble_into_the_rows() {
        let r = rows();
        let batch = batch_of(&r, 3).expect("batches");
        assert_eq!(batch.num_rows(), r.len());
        let src = r.as_le_bytes();
        assert!(src.iter().any(|&b| b != 0), "all-zero rows prove nothing");

        for row in 0..batch.num_rows() {
            let mut rebuilt = Vec::with_capacity(NODE_ROW_STRIDE);
            for col in 0..batch.num_columns() {
                let a = batch
                    .column(col)
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .expect("fixed-size binary");
                rebuilt.extend_from_slice(a.value(row));
            }
            assert_eq!(
                rebuilt,
                &src[row * NODE_ROW_STRIDE..(row + 1) * NODE_ROW_STRIDE],
                "row {row} did not reassemble"
            );
        }
    }

    /// A real dataset round-trip: write the rows, read the count back.
    #[test]
    fn rows_land_in_a_dataset_and_can_be_counted() {
        let dir = std::env::temp_dir().join(format!("blockly-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tempdir");
        let uri = dir.join("programs.lance");
        let store = LanceStore::open(uri.to_str().expect("utf-8")).expect("opens");

        let r = rows();
        store.append(&r, 1).expect("first write creates the table");
        assert_eq!(store.count().expect("counts"), r.len());
        store.append(&r, 2).expect("second write appends");
        assert_eq!(store.count().expect("counts"), r.len() * 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
