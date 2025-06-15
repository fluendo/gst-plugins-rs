# Video Encoder Stats

- `video-encoder-stats`:
  The element that collects statistics from a video encoder, and attach them onto the `GstBuffers` as metadata. It helps analyze encoding performance and quality metrics.

- `video-compare-mixer`:
  The element in charge of comparing and mixing multiple video streams. Useful for side-by-side quality comparisons or blending outputs from different encoders.

- `videoencoderstatsmeta`:
  Defines metadata structures and logic for handling video encoder statistics along the pipeline. It has been defined as `GstVideoEncoderStatsMetaAPI`.


**`video-encoder-stats`** example:
```
cargo r --example video-encoder-stats
```
