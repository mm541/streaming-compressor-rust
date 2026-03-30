/// Events emitted during compression/decompression for progress tracking.
/// Consumers receive these via `std::sync::mpsc::Sender<ProgressEvent>`.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// A fragment has started processing.
    FragmentStarted {
        idx: usize,
        total_fragments: usize,
    },

    /// A fragment has finished processing successfully.
    FragmentCompleted {
        idx: usize,
        original_size: u64,
        compressed_size: u64,
    },

    /// Incremental byte count update (for fine-grained progress bars).
    BytesProcessed(u64),

    /// A non-fatal error occurred during processing.
    Error {
        idx: usize,
        msg: String,
    },
}
