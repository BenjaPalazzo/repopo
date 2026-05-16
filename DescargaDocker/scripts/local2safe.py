#!/usr/bin/env python3
"""
local2safe — wrapper entry-point installed in the sisar/download container.

Delegates entirely to the burst2safe library's local2safe implementation.
See the source at local2safe.py in the project reference files.

Usage (as invoked by sisar-download binary):
    local2safe <burst_list.json> --all_anns --work_dir <dir>
"""

import argparse
import json
import sys
from pathlib import Path

# burst2safe must be installed in the container image
try:
    from burst2safe import utils
    from burst2safe.safe import Safe
except ImportError as exc:
    print(f"[local2safe] ERROR: burst2safe not installed: {exc}", file=sys.stderr)
    sys.exit(1)

# Import the core logic from local2safe.py (bundled alongside this script
# during the Docker build, or installed as part of burst2safe extras).
# We reproduce the two key functions here to avoid depending on a separate
# script file being present on PATH.

from datetime import datetime
from typing import cast

from burst2safe.burst_id import calculate_burstid


def burst_info_from_local(
    tiff_path: Path,
    xml_path: Path,
    slc_name: str,
    swath: str,
    polarization: str,
    burst_index: int,
) -> utils.BurstInfo:
    extractor_url = "https://sentinel1-burst.asf.alaska.edu"
    burst_url_base = f"{extractor_url}/{slc_name}/{swath}/{polarization}/{burst_index}"
    data_url = f"{burst_url_base}.tiff"
    metadata_url = f"{burst_url_base}.xml"

    manifest = utils.get_subxml_from_metadata(xml_path, "manifest", swath, polarization)
    xml_orbit_path = (
        './/{*}metadataObject[@ID="measurementOrbitReference"]'
        "/metadataWrap/xmlData/{*}orbitReference"
    )
    meta_orbit = manifest.find(xml_orbit_path)
    abs_orbit_start, _ = [int(x.text) for x in meta_orbit.findall("{*}orbitNumber")]  # type: ignore[arg-type]
    rel_orbit_start, rel_orbit_stop = [
        int(x.text) for x in meta_orbit.findall("{*}relativeOrbitNumber")  # type: ignore[arg-type]
    ]
    direction = meta_orbit.find("{*}extension/{*}orbitProperties/{*}pass").text.upper()

    product = utils.get_subxml_from_metadata(xml_path, "product", swath, polarization)
    sensing_time_str = (
        product.findall("swathTiming/burstList/burst")[burst_index].find("sensingTime").text
    )
    anx_time_str = meta_orbit.find(
        "{*}extension/{*}orbitProperties/{*}ascendingNodeTime"
    ).text
    assert sensing_time_str is not None
    assert anx_time_str is not None
    burst_id, rel_orbit = calculate_burstid(
        sensing_time_str, anx_time_str, rel_orbit_start, rel_orbit_stop, swath
    )
    info = utils.BurstInfo(
        granule="",
        slc_granule=slc_name,
        swath=swath,
        polarization=polarization,
        burst_id=burst_id,
        burst_index=burst_index,
        direction=direction,
        absolute_orbit=abs_orbit_start,
        relative_orbit=rel_orbit_start,
        date=None,
        data_url=data_url,
        data_path=tiff_path,
        metadata_url=metadata_url,
        metadata_path=xml_path,
    )
    info.add_shape_info()
    info.add_start_stop_utc()
    date_format = "%Y%m%dT%H%M%S"
    start_utc_str = datetime.strftime(cast(datetime, info.start_utc), date_format)
    info.date = datetime.strptime(
        datetime.strftime(cast(datetime, info.start_utc), date_format), date_format
    )
    info.granule = (
        f"S1_{burst_id}_{swath}_{start_utc_str}_{polarization}"
        f"_{slc_name.split('_')[-1]}-BURST"
    )
    return info


def load_burst_infos(slc_dict: dict) -> list[utils.BurstInfo]:
    valid_swaths = ["IW1", "IW2", "IW3", "EW1", "EW2", "EW3", "EW4", "EW5"]
    valid_pols = ["VV", "VH", "HV", "HH"]
    burst_infos = []
    for slc_name in slc_dict:
        slc_name = slc_name.upper()
        for swath in slc_dict[slc_name]:
            swath = swath.upper()
            if swath not in valid_swaths:
                raise ValueError(f"Invalid swath: {swath}")
            for polarization in slc_dict[slc_name][swath]:
                polarization = polarization.upper()
                if polarization not in valid_pols:
                    raise ValueError(f"Invalid polarization: {polarization}")
                burst_dict = slc_dict[slc_name][swath][polarization]
                for burst_index in burst_dict:
                    info = burst_info_from_local(
                        Path(burst_dict[burst_index]["DATA"]),
                        Path(burst_dict[burst_index]["METADATA"]),
                        slc_name,
                        swath,
                        polarization,
                        int(burst_index),
                    )
                    burst_infos.append(info)
    return burst_infos


def local2safe(
    slc_dict: dict,
    all_anns: bool = False,
    keep_files: bool = True,
    work_dir: Path | str | None = None,
) -> Path:
    from burst2safe import utils as _utils

    work_dir = _utils.optional_wd(work_dir)
    burst_infos = load_burst_infos(slc_dict)
    print(f"[local2safe] Found {len(burst_infos)} burst(s).", flush=True)
    print("[local2safe] Checking burst group validity …", flush=True)
    Safe.check_group_validity(burst_infos)
    print("[local2safe] Creating SAFE …", flush=True)
    safe = Safe(burst_infos, all_anns, work_dir)
    safe_path = safe.create_safe()
    print(f"[local2safe] SAFE created at {safe_path}", flush=True)
    if not keep_files:
        safe.cleanup()
    return safe_path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate SAFE file(s) from local burst extractor outputs"
    )
    parser.add_argument(
        "json_tree_path",
        type=Path,
        help=(
            "Path to the SLC tree JSON file "
            "{slc: {swath: {pol: {burst_index: {DATA: …, METADATA: …}}}}}"
        ),
    )
    parser.add_argument("--all_anns", action="store_true", help="Include all annotations")
    parser.add_argument("--work_dir", type=Path, help="Working / output directory")
    args = parser.parse_args()

    slc_tree = json.loads(args.json_tree_path.read_text())
    local2safe(slc_tree, args.all_anns, work_dir=args.work_dir)


if __name__ == "__main__":
    main()
