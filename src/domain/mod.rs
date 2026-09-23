//! 纯领域规则：不接触 SQLite / HTTP，全部可单元测试。

pub mod units;
pub mod raw;
pub mod normalize;
pub mod subtract;
pub mod models;
pub mod events;
pub mod state;
pub mod ids;

pub use units::{Measure, UnitKind, ConversionDiagnostic};
pub use raw::{DataPoint, RawCurve, RawRecord, RawRecords, ParsedQuantity};
pub use normalize::{normalize_record, NormalizedPoint, NormalizedCurve, NormalizeReport};
pub use subtract::{subtract_background, BackgroundPoint, BackgroundResult, SubtractStatus};
pub use models::{guinier_candidates, porod_candidates, ModelCandidate, FitKind};
pub use events::{Event, EventPayload, RawCurvePayload, MetadataPatch, BufferChoice, Decision, Artifact, ArtifactKind};
pub use state::{ProjectState, RecordView, DerivationView};
pub use ids::{content_id, content_id_bytes};
