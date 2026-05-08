#!/usr/bin/env python3
"""
download_dem — Download a Copernicus GLO-30 DEM for a given extent
and prepare it for ISCE2 processing.

Does NOT require ISCE2 to be installed.

Produces four files in the output directory:
    full_res.dem.wgs84          — raw float32 binary raster (BIP, LSB, ellipsoidal heights)
    full_res.dem.wgs84.aux.xml  — GDAL auxiliary file (written automatically by rasterio)
    full_res.dem.wgs84.vrt      — GDAL VRTRawRasterBand description of the binary layout
    full_res.dem.wgs84.xml      — ISCE2 imageFile XML (replaces fixImageXml.py output)

Usage:
    download_dem -b <west> <south> <east> <north> -o <output_dir>
    download_dem -b -69.16 -46.38 -69.04 -46.28 -o /job/dem
"""

import argparse
from pathlib import Path

import numpy as np
import rasterio
from dem_stitcher.stitcher import stitch_dem
from lxml import etree
from shapely.geometry import box

# ISCE2 version string written into the XML. The value is cosmetic — ISCE2
# does not validate it, but it must be present for the imageFile schema.
ISCE_VERSION_STRING = "Release: 2.6.3, svn-, 20230418. Current: svn-."


# ── VRT ───────────────────────────────────────────────────────────────────────

def write_vrt(dem_path: Path, width: int, height: int, geotransform: tuple) -> Path:
    """Write a VRTRawRasterBand .vrt that describes the raw float32 binary.

    The VRT format must match the working example exactly:
      - dataType="Float32", subClass="VRTRawRasterBand"
      - ByteOrder LSB, ImageOffset 0
      - PixelOffset 4 (float32 = 4 bytes)
      - LineOffset = width * 4

    Args:
        dem_path:     Path to the binary raster file.
        width:        Number of columns (pixels per line).
        height:       Number of rows (lines).
        geotransform: GDAL-style 6-tuple (x_origin, x_res, 0, y_origin, 0, y_res).

    Returns:
        Path to the written .vrt file.
    """
    vrt_path = Path(str(dem_path) + ".vrt")

    x_origin, x_res, _, y_origin, _, y_res = geotransform

    root = etree.Element(
        "VRTDataset",
        rasterXSize=str(width),
        rasterYSize=str(height),
    )

    srs = etree.SubElement(root, "SRS")
    srs.text = "EPSG:4326"

    gt = etree.SubElement(root, "GeoTransform")
    # Format to match the working example: fixed precision, no trailing zeros
    gt.text = (
        f"{x_origin:.4f}, {x_res:.9f}, 0.0, "
        f"{y_origin:.4f}, 0.0, {y_res:.9f}"
    )

    band = etree.SubElement(
        root,
        "VRTRasterBand",
        dataType="Float32",
        band="1",
        subClass="VRTRawRasterBand",
    )

    src = etree.SubElement(band, "SourceFilename")
    src.set("relativeToVRT", "1")
    src.text = dem_path.name

    etree.SubElement(band, "ByteOrder").text = "LSB"
    etree.SubElement(band, "ImageOffset").text = "0"
    etree.SubElement(band, "PixelOffset").text = "4"
    etree.SubElement(band, "LineOffset").text = str(width * 4)

    tree = etree.ElementTree(root)
    #tree.write(str(vrt_path), pretty_print=True, xml_declaration=False, encoding="unicode")
    xml_str = etree.tostring(root, pretty_print=True).decode("utf-8")
    vrt_path.write_text(xml_str, encoding="utf-8")

    return vrt_path


# ── ISCE2 XML ─────────────────────────────────────────────────────────────────

def _coord_component(name: str, doc: str, delta: float, start: float,
                     size: int) -> etree._Element:
    """Build an ISCE2 <component name="coordinateN"> element."""
    comp = etree.Element("component", name=name)
    etree.SubElement(comp, "factorymodule").text = "isceobj.Image"
    etree.SubElement(comp, "factoryname").text = "createCoordinate"
    etree.SubElement(comp, "doc").text = doc

    def prop(pname: str, value: str, pdoc: str) -> None:
        p = etree.SubElement(comp, "property", name=pname)
        etree.SubElement(p, "value").text = value
        etree.SubElement(p, "doc").text = pdoc

    # delta (pixel spacing; negative for y axis)
    prop("delta", f"{delta:.9f}".rstrip("0").rstrip(".") + ("" if "." in f"{delta:.9f}".rstrip("0") else ""), "Coordinate quantization.")

    # ending value: start + (size - 1) * delta
    end = start + (size - 1) * delta
    prop("endingvalue", str(end), "Ending value of the coordinate.")
    prop("family", "ImageCoordinate", "Instance family name")
    prop("name", "ImageCoordinate_name", "Instance name")
    prop("size", str(size), "Coordinate size.")
    prop("startingvalue", str(start), "Starting value of the coordinate.")

    return comp


def write_isce_xml(dem_path: Path, width: int, height: int,
                   geotransform: tuple) -> Path:
    """Write the ISCE2 imageFile XML sidecar for the DEM binary.

    Reproduces the output of fixImageXml.py without requiring ISCE2.

    Args:
        dem_path:     Absolute path to the binary raster file.
        width:        Number of columns.
        height:       Number of rows.
        geotransform: GDAL-style 6-tuple (x_origin, x_res, 0, y_origin, 0, y_res).

    Returns:
        Path to the written .xml file.
    """
    xml_path = Path(str(dem_path) + ".xml")

    x_origin, x_res, _, y_origin, _, y_res = geotransform

    root = etree.Element("imageFile")

    def prop(pname: str, value: str, pdoc: str = "") -> None:
        p = etree.SubElement(root, "property", name=pname)
        etree.SubElement(p, "value").text = value
        if pdoc:
            etree.SubElement(p, "doc").text = pdoc

    prop("ISCE_VERSION", ISCE_VERSION_STRING)
    prop("access_mode", "READ", "Image access mode.")
    prop("byte_order", "l", "Endianness of the image.")

    # coordinate1 = x / longitude axis (positive delta)
    root.append(_coord_component(
        name="coordinate1",
        doc="['First coordinate of a 2D image (width).']",
        delta=x_res,
        start=x_origin,
        size=width,
    ))

    # coordinate2 = y / latitude axis (negative delta)
    root.append(_coord_component(
        name="coordinate2",
        doc="Second coordinate of a 2D image (length).",
        delta=y_res,
        start=y_origin,
        size=height,
    ))

    prop("data_type", "FLOAT", "Image data type.")
    prop("family", "demimage", "Instance family name")
    prop("file_name", str(dem_path.resolve()), "Name of the image file.")
    prop("image_type", "dem", "Image type used for displaying.")
    prop("length", str(height), "Image length")
    prop("name", "demimage_name", "Instance name")
    prop("number_bands", "1", "Number of image bands.")
    prop("reference", "WGS84", "Geodetic datum")
    prop("scheme", "BIP", "Interleaving scheme of the image.")
    prop("width", str(width), "Image width")

    # xmin / xmax: longitude range
    x_end = x_origin + (width - 1) * x_res
    prop("xmax", str(x_end), "Maximum range value")
    prop("xmin", str(x_origin), "Minimum range value")

    tree = etree.ElementTree(root)
    #tree.write(str(xml_path), pretty_print=True, xml_declaration=False, encoding="unicode")
    xml_str = etree.tostring(root, pretty_print=True).decode("utf-8")
    xml_path.write_text(xml_str, encoding="utf-8")

    return xml_path


# ── Main download logic ───────────────────────────────────────────────────────

def download_dem(
    bounds: list[float],
    output_dir: Path,
    dem_name: str = "glo_30",
    buffer: float = 0.004,
) -> Path:
    """
    Download and stitch a Copernicus DEM, write all files ISCE2 requires.

    Args:
        bounds:     [west, south, east, north] in EPSG:4326
        output_dir: Target directory (created if absent)
        dem_name:   dem_stitcher dataset name ('glo_30' or 'glo_90')
        buffer:     Degree buffer around the requested extent

    Returns:
        Path to the produced binary raster (full_res.dem.wgs84)
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

    print(f"[download_dem] Stitching {dem_name} for extent {buffered} ...", flush=True)

    dem_res = 0.0002777777777777777775  # ~1 arc-second (GLO-30 native)

    dem_array, dem_profile = stitch_dem(
        buffered,
        dem_name,
        dst_ellipsoidal_height=True,  # geoid -> WGS84 ellipsoidal conversion
        dst_area_or_point="Point",
        n_threads_downloading=1,
        dst_resolution=dem_res,
    )

    dem_array[np.isnan(dem_array)] = 0.0
    height, width = dem_array.shape

    # Extract geotransform from rasterio profile transform
    t = dem_profile["transform"]
    # rasterio Affine: (a=x_res, b=0, c=x_origin, d=0, e=y_res, f=y_origin)
    geotransform = (t.c, t.a, 0.0, t.f, 0.0, t.e)
    x_origin, x_res, _, y_origin, _, y_res = geotransform

    # ── 1. Write raw float32 binary (no driver metadata embedded) ────────────
    # Write as plain float32 little-endian binary — the VRT and XML describe
    # the layout to GDAL and ISCE2 respectively, so no GDAL driver is needed.
    dem_path = output_dir / "full_res.dem.wgs84"
    print(f"[download_dem] Writing binary raster to {dem_path} ...", flush=True)
    dem_array.astype("<f4").tofile(dem_path)  # <f4 = float32 little-endian
    print(f"[download_dem] Binary written ({width}x{height} float32 LSB).", flush=True)

    # ── 2. Write VRTRawRasterBand .vrt ───────────────────────────────────────
    vrt_path = write_vrt(dem_path, width, height, geotransform)
    print(f"[download_dem] VRT written: {vrt_path}", flush=True)

    # ── 3. Write ISCE2 imageFile .xml ─────────────────────────────────────────
    xml_path = write_isce_xml(dem_path, width, height, geotransform)
    print(f"[download_dem] ISCE2 XML written: {xml_path}", flush=True)

    # Note: .aux.xml is produced by GDAL when the raster is first read via the
    # VRT. It will be created by the ISCE2 processing container on first access
    # and does not need to be generated here.

    print(f"[download_dem] DEM ready in {output_dir}", flush=True)
    print(f"[download_dem]   {dem_path.name}", flush=True)
    print(f"[download_dem]   {vrt_path.name}", flush=True)
    print(f"[download_dem]   {xml_path.name}", flush=True)

    return dem_path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Download Copernicus DEM for ISCE2 (no ISCE2 required)",
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
