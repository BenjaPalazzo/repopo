#!/usr/bin/env python3
"""
download_dem — Download a Copernicus GLO-30 DEM for a given extent
and prepare it for ISCE2 processing.

Installed in the sisar/download container at /usr/local/bin/download_dem.

Usage:
    download_dem --bounds <west> <south> <east> <north> --output <dir>
    download_dem -b -69.16 -46.38 -69.04 -46.28 -o /job/dem
"""

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio
from dem_stitcher.stitcher import stitch_dem
from lxml import etree
from shapely.geometry import box


def tag_dem_xml_as_ellipsoidal(dem_path: Path) -> Path:
    """Add the WGS84 ellipsoidal reference tag to the ISCE2 DEM XML sidecar."""
    xml_path = Path(str(dem_path) + ".xml")
    if not xml_path.exists():
        raise FileNotFoundError(f"DEM XML sidecar not found: {xml_path}")

    tree = etree.parse(str(xml_path))
    root = tree.getroot()

    ref_elem = etree.Element("property", name="reference")
    etree.SubElement(ref_elem, "value").text = "WGS84"
    etree.SubElement(ref_elem, "doc").text = "Geodetic datum"
    root.insert(0, ref_elem)

    with open(xml_path, "wb") as fh:
        fh.write(etree.tostring(root, pretty_print=True))

    return xml_path


def fix_image_xml(isce_raster_path: str) -> None:
    """Run ISCE2's fixImageXml.py to update the XML sidecar with correct metadata."""
    import subprocess, isce  # noqa: F401 – imported for side-effects (sets isce_path)

    apps = Path(isce.isce_path) / "applications"
    subprocess.check_call(
        [str(apps / "fixImageXml.py"), "-i", str(isce_raster_path), "--full"]
    )


def download_dem(
    bounds: list[float],
    output_dir: Path,
    dem_name: str = "glo_30",
    buffer: float = 0.004,
) -> Path:
    """
    Download and stitch a Copernicus DEM tile set, save as ISCE2-compatible binary.

    Args:
        bounds:     [west, south, east, north] in EPSG:4326
        output_dir: Target directory (created if absent)
        dem_name:   dem_stitcher dataset name ('glo_30' or 'glo_90')
        buffer:     Degree buffer around the requested extent

    Returns:
        Path to the produced DEM file (full_res.dem.wgs84)
    """
    output_dir.mkdir(parents=True, exist_ok=True)

    west, south, east, north = bounds
    extent_geo = box(west, south, east, north)
    buffered = list(extent_geo.buffer(buffer).bounds)
    buffered = [
        float(np.floor(buffered[0])),
        float(np.floor(buffered[1])),
        float(np.ceil(buffered[2])),
        float(np.ceil(buffered[3])),
    ]

    print(f"[download_dem] Stitching {dem_name} for extent {buffered} …", flush=True)

    # ~1 arc-second resolution (same as Copernicus GLO-30 native)
    dem_res = 0.0002777777777777777775

    dem_array, dem_profile = stitch_dem(
        buffered,
        dem_name,
        dst_ellipsoidal_height=True,
        dst_area_or_point="Point",
        n_threads_downloading=4,
        dst_resolution=dem_res,
    )

    dem_path = output_dir / "full_res.dem.wgs84"
    dem_array[np.isnan(dem_array)] = 0.0

    # Write ISCE-format binary (single-band float32, no compression)
    profile = dem_profile.copy()
    profile["nodata"] = None
    profile["driver"] = "ISCE"
    for key in ("blockxsize", "blockysize", "compress", "interleave", "tiled"):
        profile.pop(key, None)

    with rasterio.open(dem_path, "w", **profile) as ds:
        ds.write(dem_array, 1)

    print(f"[download_dem] DEM written to {dem_path}", flush=True)

    xml_path = tag_dem_xml_as_ellipsoidal(dem_path)
    print(f"[download_dem] XML sidecar tagged as ellipsoidal: {xml_path}", flush=True)

    try:
        fix_image_xml(str(dem_path))
    except Exception as exc:
        print(
            f"[download_dem] WARNING: fixImageXml.py failed ({exc}); "
            "ISCE2 may need to regenerate the XML at runtime.",
            file=sys.stderr,
        )

    return dem_path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Download Copernicus DEM for ISCE2",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "-b", "--bounds",
        nargs=4,
        type=float,
        metavar=("WEST", "SOUTH", "EAST", "NORTH"),
        required=True,
        help="Geographic bounds in EPSG:4326",
    )
    parser.add_argument(
        "-o", "--output",
        type=Path,
        required=True,
        metavar="DIR",
        help="Output directory",
    )
    parser.add_argument(
        "--dem",
        default="glo_30",
        choices=["glo_30", "glo_90"],
        help="DEM dataset name (default: glo_30)",
    )
    args = parser.parse_args()

    west, south, east, north = args.bounds
    if west >= east:
        parser.error("WEST must be less than EAST")
    if south >= north:
        parser.error("SOUTH must be less than NORTH")

    download_dem(
        bounds=[west, south, east, north],
        output_dir=args.output,
        dem_name=args.dem,
    )


if __name__ == "__main__":
    main()
