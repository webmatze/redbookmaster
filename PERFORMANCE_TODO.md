# Performance TODO

Performance issues identified during code review. Items are organized by priority.

---

## Priority 1 - Critical (Memory/Crash Potential)

_All critical items completed - see Completed section._

---

## Priority 2 - High (Noticeable Performance Impact)

### [ ] Batch write samples in concatenation
**File:** `crates/redbookmaster-lib/src/audio/concat.rs` (lines 44-47, 70-73)

**Issue:** Each `write_sample` call is an individual operation with potential buffering overhead.

**Current code:**
```rust
for _ in 0..num_samples {
    writer.write_sample(0i16).map_err(|e| ...)?;
    writer.write_sample(0i16).map_err(|e| ...)?;
}
```

**Recommendation:** Batch write samples using a buffer:
```rust
const BUFFER_SIZE: usize = 8192;
let silence_buffer = vec![0i16; BUFFER_SIZE];
for chunk in silence_buffer.chunks(BUFFER_SIZE) {
    writer.write_samples(chunk)?;
}
```

---

### [ ] Update VecModel rows in-place instead of recreating
**File:** `crates/redbookmaster-gui/src/main.rs` (multiple locations)

**Issue:** Every time tracks are updated (title change, pregap change, etc.), a completely new `VecModel` is created and set.

**Current code:**
```rust
let tracks: Vec<TrackData> = state.tracks_to_model();
let model = Rc::new(slint::VecModel::from(tracks));
app.set_tracks(model.into());
```

**Recommendation:** Update existing model rows instead of recreating:
```rust
if let Some(row) = model.row_data(index) {
    let mut updated = row;
    updated.title = new_title.into();
    model.set_row_data(index, updated);
}
```

---

### [ ] Move track file reading to background thread
**File:** `crates/redbookmaster-gui/src/main.rs` (line 521)

**Issue:** When adding tracks, `read_wav_info()` is called synchronously for each file in the UI callback. For multiple files or large files on slow storage, this blocks the UI.

**Recommendation:** Move file processing to background thread, similar to how export is handled.

---

### [ ] Fix double file open in LoadAndPlay
**File:** `crates/redbookmaster-gui/src/player/engine.rs` (lines 256-293)

**Issue:** Files are opened twice during LoadAndPlay - once to get duration, once to decode for playback.

**Recommendation:** Cache the decoded source or use rodio's `Decoder::total_duration()` without consuming the source.

---

### [ ] Debounce auto-save
**File:** `crates/redbookmaster-gui/src/main.rs` (lines 179-185)

**Issue:** Auto-save is called after every metadata change (title edit, pregap change, etc.). Each save involves JSON serialization and file write.

**Recommendation:** Debounce auto-save (e.g., save at most once per second, or on focus loss).

---

## Priority 3 - Medium (Optimization Opportunities)

### [ ] Consider Path element for waveform rendering
**File:** `crates/redbookmaster-gui/ui/main.slint` (lines 138-158)

**Issue:** The waveform view creates 500 individual `Rectangle` elements (one per peak bin). Each element requires layout calculations, property binding evaluation, and rendering overhead.

**Note:** The library already has a `peaks_to_svg_path()` function that could be used.

**Recommendation:** Use a `Path` element with SVG path data for waveform rendering instead of 500 rectangles.

---

### [ ] Use blocking receive in audio thread when idle
**File:** `crates/redbookmaster-gui/src/player/engine.rs` (line 206)

**Issue:** The audio thread wakes up every 50ms to check for commands, even when not playing (~20 thread wakeups per second when idle).

**Recommendation:**
```rust
let timeout = if state.is_playing.load(Ordering::Relaxed) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(60)  // Long timeout when idle
};
```

---

### [ ] Reduce position update frequency
**File:** `crates/redbookmaster-gui/src/player/engine.rs` (lines 458-460)

**Issue:** Position updates are sent every 50ms loop iteration, creating 20 events per second. Most of these may not be processed by the UI.

**Recommendation:** Send position updates at a lower rate (e.g., 200ms) or only when the value changes significantly.

---

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

## Notes

- LTO is already enabled in release profile
- Async waveform loading with cancellation is well implemented
- Crossbeam channels are used appropriately
- Atomic state is used correctly for thread safety
- Weak references are properly used in Slint callbacks
