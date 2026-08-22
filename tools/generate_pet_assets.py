#!/usr/bin/env python3
"""Convert the canonical Ripple pet GIF frames into LVGL I8 assets.

The source frames, ordering, and timing are treated as immutable. The only
transformation is a proportional 384x416 -> 144x156 display-size conversion.
"""

from pathlib import Path
import subprocess
import sys
import tempfile

from PIL import Image


PROJECT_DIR = Path(__file__).resolve().parents[1]
SOURCE_DIR = PROJECT_DIR / "assets/pet-gifs"
OUTPUT_DIR = PROJECT_DIR / "main/pet_assets"
CONVERTER = PROJECT_DIR / "managed_components/lvgl__lvgl/scripts/LVGLImage.py"
OUTPUT_SIZE = (144, 156)

EXPECTED = {
    "failed": [140, 140, 140, 140, 140, 140, 140, 240],
    "idle": [280, 110, 110, 140, 140, 320],
    "running": [120, 120, 120, 120, 120, 220],
    "waiting": [150, 150, 150, 150, 150, 260],
    "waving": [140, 140, 140, 280],
}


def main() -> None:
    if not CONVERTER.exists():
        raise SystemExit("LVGL converter missing; run the ESP-IDF component build first")
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for old_asset in OUTPUT_DIR.glob("passport_pet_*.c"):
        old_asset.unlink()

    with tempfile.TemporaryDirectory(prefix="passport-pet-") as temporary:
        temporary_dir = Path(temporary)
        for state, expected_durations in EXPECTED.items():
            source = SOURCE_DIR / f"starry-avatar-{state}.gif"
            image = Image.open(source)
            durations = []
            if image.size != (384, 416) or image.n_frames != len(expected_durations):
                raise SystemExit(f"canonical pet asset shape changed: {source}")

            for frame_index in range(image.n_frames):
                image.seek(frame_index)
                durations.append(image.info.get("duration"))
                frame = image.convert("RGBA").resize(OUTPUT_SIZE, Image.Resampling.LANCZOS)
                png = temporary_dir / f"{state}_{frame_index}.png"
                frame.save(png)
                name = f"passport_pet_{state}_{frame_index}"
                subprocess.run(
                    [sys.executable, str(CONVERTER), "--ofmt", "C", "--cf", "I8",
                     "--name", name, "-o", str(OUTPUT_DIR), str(png)],
                    check=True,
                )
                generated = OUTPUT_DIR / f"{name}.c"
                generated.write_text(generated.read_text().rstrip() + "\n")

            if durations != expected_durations:
                raise SystemExit(f"canonical pet animation timing changed: {source}")

    print(f"generated {sum(len(value) for value in EXPECTED.values())} immutable pet frames")


if __name__ == "__main__":
    main()
