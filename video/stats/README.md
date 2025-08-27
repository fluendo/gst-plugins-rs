# Video Encoder Stats

- `video-encoder-stats`:
  The element that collects statistics from a video encoder, and attach them onto the `GstBuffers` as metadata. It helps analyze encoding performance and quality metrics.

- `video-compare-mixer`:
  The element in charge of comparing and mixing multiple video streams. Useful for side-by-side quality comparisons or blending outputs from different encoders.

  User can change the video showed using the next keys:

    1: Only first video
    2: Only second video
    3: First and second videos split mode (default)
    4: First and second videos side by side mode (default)
    5: Move side by side border left
    6: Move side by side border right

Also click in the botton of the video can be done to change the side by side border

User can change the video player zoom using the next keys:

    +: Zoom in
    -: Zoom out
    Up/Down/Right/Left: Move the frame
    r: reset the zoom position
    R: reset the zoom

Also mouse navigation events can be used for a better UX.

- `videoencoderstatsmeta`:
  Defines metadata structures and logic for handling video encoder statistics along the pipeline. It has been defined as `GstVideoEncoderStatsMetaAPI`.


**`video-encoder-stats`** example:
```
cargo r --example video-encoder-stats
```
