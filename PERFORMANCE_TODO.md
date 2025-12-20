# Performance TODO

Performance issues identified during code review. Items are organized by priority.

---

## Priority 1 - Critical (Memory/Crash Potential)

### [ ] Implement streaming for waveform extraction
**File:** `crates/redbookmaster-lib/src/audio/waveform.rs` (lines 104-116)

**Issue:** The `extract_peaks` function loads the entire audio file into memory as a `Vec<i32>` or `Vec<f32>` before processing. For a 79-minute CD-quality audio file (44100 Hz x 2 channels x 16-bit x 79 min), this allocates approximately 830 MB of memory.

**Current code:**
```rust
let samples: Vec<i32> = reader.into_samples::<i32>()
    .filter_map(|s| s.ok())
    .collect();
```

**Recommendation:** Use streaming/chunked processing instead of collecting all samples:
```rust
const CHUNK_SIZE: usize = 65536;
let mut peaks = Vec::with_capacity(target_peaks);
let samples_per_peak = (total_samples as usize / target_peaks).max(1);
// Stream through samples in chunks...
```

---

### [ ] Implement streaming for audio conversion
**File:** `crates/redbookmaster-lib/src/audio/convert.rs` (lines 63-89)

**Issue:** The conversion pipeline:
1. Loads entire file as `Vec<f64>`
2. Creates a new `Vec<f64>` for channel conversion
3. Creates another new `Vec<f64>` for resampling
4. Creates deinterleaved channel data (more allocations)
5. Creates reinterleaved output

For a 79-minute file at f64, this can require 1.6+ GB of memory just for the initial load, with peak usage of 3-4 GB during conversion.

**Recommendation:** Implement streaming conversion with fixed-size buffers.

---

### [ ] Add LRU eviction to waveform cache
**File:** `crates/redbookmaster-gui/src/main.rs` (lines 117-118)

**Issue:** The waveform cache grows unbounded. Each cached track stores `WAVEFORM_BINS * 16 = 8000` peak pairs (approximately 128KB per track in memory). With 99 possible tracks, this could use ~12.7 MB just for peak data.

**Recommendation:** Implement LRU cache with a maximum size limit (e.g., 10-20 tracks).

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

### [ ] Pre-allocate buffers in resampler
**File:** `crates/redbookmaster-lib/src/audio/convert.rs` (lines 256-266)

**Issue:** New vectors are allocated for every chunk during resampling. For a large file processed in 1024-sample chunks, this creates thousands of allocations.

**Recommendation:** Pre-allocate reusable buffers outside the loop.

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

_Move items here when done._

---

## Notes

- LTO is already enabled in release profile
- Async waveform loading with cancellation is well implemented
- Crossbeam channels are used appropriately
- Atomic state is used correctly for thread safety
- Weak references are properly used in Slint callbacks
