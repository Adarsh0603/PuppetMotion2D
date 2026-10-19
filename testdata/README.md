# Test drawings

The same set is used at every step so quality changes are visible over time.

## What to add (15–20 images, PNG or JPG)
- **Kids' sketches** – pencil/marker on paper, ideally some with gaps in outlines and lined paper.
- **Phone photos of paper drawings** – uneven lighting, shadows at the edges.
- **Vector clip art** – flat solid background, and some with a transparent background.
- **Webcomic characters** – colored, with outlines, possibly a textured or gradient background.

Name files `<category>_<short-name>.png`, e.g. `kid_dragon.jpg`, `clipart_robot.png`.
The `synthetic_*.png` files are generated stand-ins; keep them as a regression baseline.

## Decision gate (Phase 0)
1. `cargo run -p anim-core --example batch_fg -- testdata` → review `testdata/out/*_fg.png`
   (magenta = foreground, darkened = background).
2. Open each drawing in the app, draw a box around one arm, judge the arm mask.
3. Record results in `RESULTS.md`. Below ~70% acceptable → add MobileSAM (via `ort`) before Phase 1.

| file | whole-figure mask | arm mask | notes |
|------|-------------------|----------|-------|
