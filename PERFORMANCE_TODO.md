# Performance TODO

Performance issues identified during code review. Items are organized by priority.

---

## Priority 1 - Critical (Memory/Crash Potential)

_All critical items completed - see Completed section._

---

## Priority 2 - High (Noticeable Performance Impact)

_All high priority items completed - see Completed section._

---

## Priority 3 - Medium (Optimization Opportunities)

### [ ] Investigate SIMD for peak extraction
**File:** `crates/redbookmaster-lib/src/audio/waveform.rs` (lines 144-154)

**Issue:** Peak extraction uses scalar operations. Modern CPUs can process 4-8 samples simultaneously with SIMD.

**Recommendation:** Use SIMD intrinsics or a crate like `wide` for vectorized min/max operations.

---

### [ ] Reuse allocation in get_peaks_for_range
**File:** `crates/redbookmaster-lib/src/audio/waveform.rs` (lines 34-73)

**Issue:** Every zoom/scroll operation calls this function, allocating a new vector of peaks.

**Recommendation:** Take a mutable slice as parameter to reuse allocation.

---

### [ ] Reduce track title cloning
**File:** `crates/redbookmaster-gui/src/main.rs` (lines 192-202)

**Issue:** Every call to `tracks_to_model()` clones all track titles and creates new `SharedString` instances.

**Recommendation:** Consider caching or reducing frequency of model updates.

---

## Completed

### [x] Implement streaming for waveform extraction (Priority 1 - Critical)
**File:** `crates/redbookmaster-lib/src/audio/waveform.rs`

Implemented streaming peak extraction using `extract_peaks_streaming_int` and `extract_peaks_streaming_float` functions that process samples one at a time instead of loading the entire file into memory.

---

### [x] Implement streaming for audio conversion (Priority 1 - Critical)
**File:** `crates/redbookmaster-lib/src/audio/convert.rs`

Implemented streaming conversion with:
- `convert_streaming_no_resample`: Memory-efficient conversion when resampling is not needed
- `convert_with_resampling`: Chunked processing with pre-allocated buffers for resampling
- `create_sample_iterator`: Streaming sample reader that converts to f64
- `write_chunk_to_wav`: Writes samples in chunks with dithering

This also fixed the "Pre-allocate buffers in resampler" issue (Priority 3) by pre-allocating channel buffers outside the processing loop.

---

### [x] Add LRU eviction to waveform cache (Priority 1 - Critical)
**File:** `crates/redbookmaster-gui/src/main.rs`

Implemented `LruWaveformCache` struct with:
- Maximum size limit of 20 tracks
- LRU eviction when capacity is reached
- `peek()` for read-only access without updating order
- `get()` for access that updates LRU order
- `contains_key()` for existence checks

---

### [x] Batch write samples in concatenation (Priority 2 - High)
**File:** `crates/redbookmaster-lib/src/audio/concat.rs`

Implemented batch processing for both silence writing and track audio writing:
- Pre-allocates a buffer of 8192 samples
- Reads and writes in batches instead of one sample at a time
- Reduces overhead from individual function calls and error mapping

---

### [x] Fix double file open in LoadAndPlay (Priority 2 - High)
**File:** `crates/redbookmaster-gui/src/player/engine.rs`

Fixed by reusing the same decoder for both getting duration and playback:
- `total_duration()` doesn't consume the decoder, so we can use the same source
- Eliminated redundant file open and decode operations

---

### [x] Debounce auto-save (Priority 2 - High)
**File:** `crates/redbookmaster-gui/src/main.rs`

Implemented debounced auto-save with:
- `has_pending_save` and `last_modification_time` fields in AppState
- `auto_save()` now marks changes as pending instead of saving immediately
- `flush_pending_save()` performs the actual save after 1 second debounce
- Timer checks for pending saves every ~1 second
- `force_save()` called on app exit to save any remaining changes

---

### [x] Update VecModel rows in-place instead of recreating (Priority 2 - High)
**File:** `crates/redbookmaster-gui/src/main.rs`

Implemented in-place VecModel updates:
- Added `tracks_model: Option<Rc<VecModel<TrackData>>>` to AppState for persistent model reference
- `initialize_tracks_model()`: Creates model and stores reference for later updates
- `update_track_title_in_model()`: Updates title in-place using `set_row_data()`
- `update_track_pregap_in_model()`: Updates pregap in-place using `set_row_data()`
- `remove_and_renumber_model()`: Removes row and renumbers remaining tracks in-place
- Track title and pregap edits now update single rows instead of rebuilding entire model

---

### [x] Move track file reading to background thread (Priority 2 - High)
**File:** `crates/redbookmaster-gui/src/main.rs`, `crates/redbookmaster-gui/ui/main.slint`

Implemented async file reading following the waveform worker pattern:
- Thread-safe containers: `add_tracks_pending`, `add_tracks_result`, `add_tracks_worker_active`
- `on_add_tracks` callback now spawns worker thread instead of reading files synchronously
- Worker thread reads all file metadata in background using `read_wav_info()`
- Timer polls for results and processes them on UI thread
- Added `adding-tracks` UI property for loading state
- File read errors now shown via `show_error_dialog()` instead of silent stderr logging
- UI stays responsive during file reading operations

---

### [x] Use Path element for waveform rendering (Priority 3 - Medium)
**File:** `crates/redbookmaster-gui/ui/main.slint`, `crates/redbookmaster-gui/src/main.rs`

Replaced 500 individual Rectangle elements with a single Path element:
- `WaveformView` component now uses `waveform-path: string` and `has-waveform: bool` properties
- Single Path element with SVG commands replaces HorizontalLayout with 500 Rectangles
- Uses existing `peaks_to_svg_path()` function to generate SVG path data
- Viewbox mapping (500x100) scales path to actual element size
- Reduced layout calculations, property bindings, and rendering overhead
- Removed unused `WaveformPeak` struct from Slint

---

### [x] Use dynamic timeout in audio thread when idle (Priority 3 - Medium)
**File:** `crates/redbookmaster-gui/src/player/engine.rs`

Implemented dynamic timeout based on playback state:
- When playing: 50ms timeout for responsive position updates
- When idle/paused: 1 second timeout to reduce thread wakeups
- Reduces idle thread wakeups from 20/second to 1/second (95% reduction)
- Commands are still processed immediately when received

---

### [x] Reduce position update frequency (Priority 3 - Medium)
**File:** `crates/redbookmaster-gui/src/player/engine.rs`

Implemented threshold-based position event sending:
- Only sends `PlayerEvent::Position` when position changes by ≥100ms
- Atomic state is still updated every iteration for accurate seeking
- Reduces position events from 20/sec to ~10/sec (50% reduction)
- Reset threshold tracking on seek and track finish for immediate UI feedback
- UI progress bar remains smooth while reducing channel traffic

---

## Notes

- LTO is already enabled in release profile
- Async waveform loading with cancellation is well implemented
- Crossbeam channels are used appropriately
- Atomic state is used correctly for thread safety
- Weak references are properly used in Slint callbacks
