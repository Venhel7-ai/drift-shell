//! VESA CVT modeline synthesis via `libdisplay-info`.
//!
//! Standard CVT (no reduced blanking) is the only variant; reduced blanking
//! cannot drive CRTs (porches too short for electron-beam retrace) and the
//! EDID-listed modes already cover modern digital displays. If a future user
//! needs CVT-R, expose it as an opt-in per-output config field.

use std::iter::zip;

use drm_ffi::drm_sys::drm_mode_modeinfo;
use drm_ffi::{DRM_MODE_FLAG_NHSYNC, DRM_MODE_FLAG_PVSYNC, DRM_MODE_TYPE_USERDEF};

/// Synthesize a VESA CVT modeline. Returns the raw FFI modeinfo (consumer
/// wraps via `drm::control::Mode::from(raw)`).
pub fn synth_cvt(w: u16, h: u16, refresh_hz: u32) -> Result<drm_mode_modeinfo, &'static str> {
    if w == 0 || h == 0 {
        return Err("CVT: width and height must be non-zero");
    }
    if refresh_hz == 0 {
        return Err("CVT: refresh rate must be non-zero");
    }
    if w < 64 || h < 64 {
        return Err("CVT: width/height too small (min 64)");
    }
    if refresh_hz > 500 {
        return Err("CVT: refresh rate too high (max 500 Hz)");
    }

    let options = libdisplay_info::cvt::Options {
        red_blank_ver: libdisplay_info::cvt::ReducedBlankingVersion::None,
        h_pixels: w as i32,
        v_lines: h as i32,
        ip_freq_rqd: refresh_hz as f64,
        video_opt: false,
        vblank: 0.0,
        additional_hblank: 0,
        early_vsync_rqd: false,
        int_rqd: false,
        margins_rqd: false,
    };
    let t = libdisplay_info::cvt::Timing::compute(options);

    let hsync_start = (w as u32 + t.h_front_porch as u32) as u16;
    let hsync_end = hsync_start.saturating_add(t.h_sync as u16);
    let htotal = hsync_end.saturating_add(t.h_back_porch as u16);

    let vsync_start = (t.v_lines_rnd as u32 + t.v_front_porch as u32) as u16;
    let vsync_end = vsync_start.saturating_add(t.v_sync as u16);
    let vtotal = vsync_end.saturating_add(t.v_back_porch as u16);

    if htotal == 0 || vtotal == 0 || hsync_start <= w || vsync_start <= h {
        return Err("CVT: degenerate timing output");
    }

    let clock = (t.act_pixel_freq * 1000.0).round() as u32;
    let vrefresh = t.act_frame_rate.round() as u32;

    let name = format!("{w}x{h}@{:.2}", t.act_frame_rate);

    Ok(drm_mode_modeinfo {
        clock,
        hdisplay: w,
        hsync_start,
        hsync_end,
        htotal,
        hskew: 0,
        vdisplay: h,
        vsync_start,
        vsync_end,
        vtotal,
        vscan: 0,
        vrefresh,
        flags: DRM_MODE_FLAG_NHSYNC | DRM_MODE_FLAG_PVSYNC,
        type_: DRM_MODE_TYPE_USERDEF,
        name: modeinfo_name_slice(&name),
    })
}

/// Pack a mode name into the 32-byte NUL-padded array DRM expects.
/// `c_char` is `i8` on x86_64 and `u8` on aarch64, so the byte-by-byte
/// `as _` cast is required for portability.
fn modeinfo_name_slice(name: &str) -> [core::ffi::c_char; 32] {
    let mut out: [core::ffi::c_char; 32] = [0; 32];
    for (dst, src) in zip(&mut out[..31], name.as_bytes()) {
        *dst = *src as _;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cvt_1920x1080_60_known_clock() {
        // Cross-checked against the `cvt 1920 1080 60` reference tool.
        let m = synth_cvt(1920, 1080, 60).unwrap();
        assert_eq!(m.clock, 173_000);
        assert_eq!(m.hdisplay, 1920);
        assert_eq!(m.vdisplay, 1080);
    }

    #[test]
    fn cvt_1152x864_100_crt_scenario() {
        // Reporter's exact bug case.
        let m = synth_cvt(1152, 864, 100).unwrap();
        assert!(
            m.clock > 130_000 && m.clock < 160_000,
            "clock = {}",
            m.clock
        );
        assert_eq!(m.hdisplay, 1152);
        assert_eq!(m.vdisplay, 864);
    }

    #[test]
    fn cvt_rejects_zero_inputs() {
        assert!(synth_cvt(0, 1080, 60).is_err());
        assert!(synth_cvt(1920, 0, 60).is_err());
        assert!(synth_cvt(1920, 1080, 0).is_err());
    }

    #[test]
    fn cvt_rejects_excessive_refresh() {
        assert!(synth_cvt(1920, 1080, 501).is_err());
    }

    #[test]
    fn cvt_rejects_tiny_dimensions() {
        assert!(synth_cvt(32, 1080, 60).is_err());
        assert!(synth_cvt(1920, 32, 60).is_err());
    }

    #[test]
    fn cvt_userdef_type_set() {
        let m = synth_cvt(1920, 1080, 60).unwrap();
        assert_eq!(m.type_, DRM_MODE_TYPE_USERDEF);
    }

    #[test]
    fn cvt_name_includes_actual_refresh() {
        // libdisplay-info reports the achieved refresh, not the requested one,
        // and our name encodes that for easier debugging from kernel logs.
        let m = synth_cvt(1920, 1080, 60).unwrap();
        // c_char is u8 on aarch64 (cast flagged as redundant) but i8 on
        // x86_64, where it's required.
        #[allow(clippy::unnecessary_cast)]
        let bytes: Vec<u8> = m.name.iter().map(|&b| b as u8).collect();
        let nul = bytes.iter().position(|&b| b == 0).expect("NUL-terminated");
        let s = std::str::from_utf8(&bytes[..nul]).unwrap();
        assert!(s.starts_with("1920x1080@"), "name = {s:?}");
    }

    #[test]
    fn cvt_timings_self_consistent() {
        let m = synth_cvt(2560, 1440, 75).unwrap();
        assert!(m.hsync_start < m.hsync_end);
        assert!(m.hsync_end <= m.htotal);
        assert!(m.hdisplay < m.hsync_start);
        assert!(m.vsync_start < m.vsync_end);
        assert!(m.vsync_end <= m.vtotal);
        assert!(m.vdisplay < m.vsync_start);
    }
}
