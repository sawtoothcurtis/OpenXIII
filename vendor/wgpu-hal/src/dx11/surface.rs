//! Screen: the window's DXGI swap chain.
//!
//! - `configure`: create (or ResizeBuffers if it exists) a swap chain for `self.hwnd`
//!   with `self.factory`: 2 buffers, DXGI_SWAP_EFFECT_FLIP_DISCARD if available (Windows 10),
//!   otherwise DXGI_SWAP_EFFECT_DISCARD; format `conv::format_dxgi(config.format)` without srgb (the
//!   srgb view, if any, is made by the texture view); MakeWindowAssociation(DXGI_MWA_NO_ALT_ENTER)
//!   (OpenXIII handles fullscreen itself). `sync_interval` = 1 for Fifo/AutoVsync, 0 otherwise.
//! - `unconfigure`: release the swap chain.
//! - `acquire_texture`: return the current back buffer as a `SurfaceTexture` (usage COLOR_TARGET
//!   | COPY_DST, plus COPY_SRC if the configuration asks for it; a single layer, one mip).
//! - `present_value` (called by `Queue::present`): Present(sync_interval, flags), handling of
//!   DXGI_ERROR_DEVICE_REMOVED -> SurfaceError::Lost, and occlusion -> Ok.
//!
//! No DXGI exclusive fullscreen: OpenXIII uses a borderless window.

use core::time::Duration;

use windows::{
    core::Interface as _,
    Win32::{
        Foundation,
        Graphics::{Direct3D11 as d3d11, Dxgi},
    },
};

use super::{conv, Api, Device, Fence, Surface, SurfaceTexture, SwapChain, Texture};

/// Number of buffers in FLIP mode (minimum required by DXGI for the FLIP_* effects).
const BUFFERS_FLIP: u32 = 2;

/// Private data of the swap chain: usages beyond COLOR_TARGET | COPY_DST requested by the
/// configuration (COPY_SRC), read back by acquire_texture (`SwapChain` has no field for them).
const GUID_USAGES_IN_MORE: windows::core::GUID = windows::core::GUID::from_u128(0x6f78_6969_6478_3131_b9e0_7573_6167_6573);

/// Usages of the screen image: always target and copy destination, copy source if requested.
fn usages_in_more(config: &crate::SurfaceConfiguration) -> u32 {
    (config.usage & wgt::TextureUses::COPY_SRC).bits() as u32
}

impl Surface {
    /// Does the factory allow DXGI_PRESENT_ALLOW_TEARING (DXGI 1.5, recent Windows 10)?
    fn tear_allowed(&self) -> bool {
        let Some(f5) = self.factory.as_factory5() else { return false };
        let mut allowed = Foundation::FALSE;
        let hr = unsafe {
            f5.CheckFeatureSupport(
                Dxgi::DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                <*mut _>::cast(&mut allowed),
                size_of_val(&allowed) as u32,
            )
        };
        hr.is_ok() && allowed.as_bool()
    }

    /// Creates a new swap chain (the old one must already be released: a single FLIP chain per window).
    unsafe fn create_swap_chain(
        &self,
        device: &Device,
        format: Dxgi::Common::DXGI_FORMAT,
        extent: wgt::Extent3d,
        without_vsync: bool,
    ) -> Result<Dxgi::IDXGISwapChain1, crate::SurfaceError> {
        // The factory of the device's adapter, not the instance's: after a display mode
        // change (exclusive fullscreen set by winit before opening), the instance's
        // factory is no longer up to date and any swap chain creation returns DXGI_ERROR_INVALID_CALL.
        let factory_value = factory_of(device)?;
        let mut desc = Dxgi::DXGI_SWAP_CHAIN_DESC1 {
            Width: extent.width,
            Height: extent.height,
            Format: format,
            Stereo: false.into(),
            SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: Dxgi::DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: BUFFERS_FLIP,
            Scaling: Dxgi::DXGI_SCALING_STRETCH,
            SwapEffect: Dxgi::DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: Dxgi::Common::DXGI_ALPHA_MODE_IGNORE,
            // The ALLOW_TEARING flag is set at creation when the factory allows it:
            // ResizeBuffers cannot add it later, and it changes nothing as long as Present
            // does not request DXGI_PRESENT_ALLOW_TEARING.
            Flags: if self.tear_allowed() { Dxgi::DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32 } else { 0 },
        };
        // Without vsync (the presentation the swap chain is created for) and without tearing
        // allowed (old drivers): in the FLIP model, Present(0) stays synced to the screen in
        // fullscreen; the old DISCARD model does not (the original also presents by copy).
        let force = false;
        if force || (without_vsync && desc.Flags == 0) {
            log::warn!("Direct3D 11: no vsync and tearing not allowed, legacy DISCARD model");
            desc.SwapEffect = Dxgi::DXGI_SWAP_EFFECT_DISCARD;
            desc.BufferCount = 1;
            desc.Flags = 0;
        }
        match unsafe { factory_value.CreateSwapChainForHwnd(&device.shared.device, self.hwnd, &desc, None, None) } {
            Ok(chain) => Ok(chain),
            Err(err) => {
                // Before Windows 10: no FLIP_DISCARD. Old model, a single buffer, without tearing.
                log::warn!("Direct3D 11: FLIP_DISCARD swap chain refused ({err}), falling back to DISCARD");
                desc.SwapEffect = Dxgi::DXGI_SWAP_EFFECT_DISCARD;
                desc.BufferCount = 1;
                desc.Flags = 0;
                match unsafe { factory_value.CreateSwapChainForHwnd(&device.shared.device, self.hwnd, &desc, None, None) } {
                    Ok(chain) => Ok(chain),
                    Err(err) => {
                        // Last attempt: the old DXGI 1.0 creation (window in exclusive fullscreen
                        // set by winit, where CreateSwapChainForHwnd returns INVALID_CALL).
                        log::warn!("Direct3D 11: DISCARD swap chain refused ({err}), trying IDXGIFactory::CreateSwapChain");
                        let prior = Dxgi::DXGI_SWAP_CHAIN_DESC {
                            BufferDesc: Dxgi::Common::DXGI_MODE_DESC { Width: extent.width, Height: extent.height, Format: format, ..Default::default() },
                            SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            BufferUsage: Dxgi::DXGI_USAGE_RENDER_TARGET_OUTPUT,
                            BufferCount: 1,
                            OutputWindow: self.hwnd,
                            Windowed: true.into(),
                            SwapEffect: Dxgi::DXGI_SWAP_EFFECT_DISCARD,
                            Flags: 0,
                        };
                        let mut chain = None;
                        let r = unsafe { factory_value.CreateSwapChain(&device.shared.device, &prior, &mut chain) };
                        match (r.ok(), chain) {
                            (Ok(()), Some(c)) => c.cast::<Dxgi::IDXGISwapChain1>().map_err(|_| crate::SurfaceError::Other("IDXGISwapChain1")),
                            (r, _) => {
                                log::error!("Direct3D 11: cannot create the swap chain: {r:?}");
                                Err(crate::SurfaceError::Other("CreateSwapChain"))
                            }
                        }
                    }
                }
            }
        }
    }
}

/// DXGI factory that is the parent of the device's adapter (always up to date for this device).
fn factory_of(device: &Device) -> Result<Dxgi::IDXGIFactory2, crate::SurfaceError> {
    let r: windows::core::Result<Dxgi::IDXGIFactory2> = (|| {
        let dxgi = device.shared.device.cast::<Dxgi::IDXGIDevice>()?;
        let adapter_value = unsafe { dxgi.GetAdapter() }?;
        unsafe { adapter_value.GetParent() }
    })();
    r.map_err(|err| {
        log::error!("Direct3D 11: DXGI factory of the device not found: {err}");
        crate::SurfaceError::Other("IDXGIAdapter::GetParent")
    })
}

/// The back buffer must no longer be bound to the context before ResizeBuffers or the destruction of the
/// swap chain; Flush immediately releases the destroyed views that the driver still holds.
unsafe fn unbind_targets(device: &Device) {
    // Kept views of the back buffer released (ResizeBuffers fails if one remains).
    device.shared.views_back.lock().clear();
    device.shared.buffer_back.store(0, core::sync::atomic::Ordering::Release);
    let ctx = device.shared.context.lock();
    device.shared.targets_generation.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
    unsafe {
        ctx.OMSetRenderTargets(None, None);
        ctx.Flush();
    }
}

impl crate::Surface for Surface {
    type A = Api;

    unsafe fn configure(&self, device: &Device, config: &crate::SurfaceConfiguration) -> Result<(), crate::SurfaceError> {
        let format = conv::format_dxgi(config.format.remove_srgb_suffix());
        let mut chain = self.swap_chain.write();
        log::info!(
            "Direct3D 11: configure {:?} {}x{} {:?} (existing swap chain: {:?})",
            config.format, config.extent.width, config.extent.height, config.present_mode,
            chain.as_ref().map(|c| (c.format, c.size.width, c.size.height))
        );

        let raw: Dxgi::IDXGISwapChain = match chain.take() {
            // Same format: the swap chain is kept, ResizeBuffers only if the size changes.
            Some(previous_one) if previous_one.format.remove_srgb_suffix() == config.format.remove_srgb_suffix() => {
                let SwapChain { raw, back_buffer, size, .. } = previous_one;
                if size != config.extent {
                    drop(back_buffer);
                    unsafe { unbind_targets(device) };
                    // Number of buffers and creation flags kept (0 = unchanged; the flags
                    // must stay those of the creation, ALLOW_TEARING included).
                    let desc = unsafe { raw.GetDesc() }.map_err(|err| {
                        log::error!("Direct3D 11: swap chain GetDesc: {err}");
                        crate::SurfaceError::Other("IDXGISwapChain::GetDesc")
                    })?;
                    unsafe {
                        raw.ResizeBuffers(
                            0,
                            config.extent.width,
                            config.extent.height,
                            format,
                            Dxgi::DXGI_SWAP_CHAIN_FLAG(desc.Flags as i32),
                        )
                    }
                    .map_err(|err| {
                        log::error!("Direct3D 11: ResizeBuffers: {err}");
                        crate::SurfaceError::Other("window is in use")
                    })?;
                }
                raw
            }
            // Other format (or first time): new swap chain, the old one released first.
            previous_one => {
                if previous_one.is_some() {
                    drop(previous_one);
                    unsafe { unbind_targets(device) };
                }
                let without_vsync = !matches!(
                    config.present_mode,
                    wgt::PresentMode::Fifo | wgt::PresentMode::AutoVsync | wgt::PresentMode::FifoRelaxed
                );
                let new_chain = unsafe { self.create_swap_chain(device, format, config.extent, without_vsync) }?;
                // DXGI must not switch to fullscreen by itself on Alt+Enter.
                if let Err(err) = factory_of(device).and_then(|f| unsafe { f.MakeWindowAssociation(self.hwnd, Dxgi::DXGI_MWA_NO_ALT_ENTER) }.map_err(|_| crate::SurfaceError::Other("MakeWindowAssociation"))) {
                    log::warn!("Direct3D 11: MakeWindowAssociation: {err}");
                }
                new_chain.into()
            }
        };

        // Number of frames the GPU may queue ahead (DXGI device setting, 1 to 16).
        match device.shared.device.cast::<Dxgi::IDXGIDevice1>() {
            Ok(dxgi_device) => {
                if let Err(err) = unsafe { dxgi_device.SetMaximumFrameLatency(config.maximum_frame_latency.clamp(1, 16)) } {
                    log::warn!("Direct3D 11: SetMaximumFrameLatency: {err}");
                }
            }
            Err(err) => log::warn!("Direct3D 11: IDXGIDevice1 missing: {err}"),
        }

        let desc = unsafe { raw.GetDesc() }.map_err(|err| {
            log::error!("Direct3D 11: swap chain GetDesc: {err}");
            crate::SurfaceError::Other("IDXGISwapChain::GetDesc")
        })?;
        let sync_interval = match config.present_mode {
            wgt::PresentMode::Fifo | wgt::PresentMode::AutoVsync | wgt::PresentMode::FifoRelaxed => 1,
            _ => 0,
        };
        // Tearing only without vsync, in the FLIP model, and if the swap chain was created for it.
        let flip = desc.SwapEffect == Dxgi::DXGI_SWAP_EFFECT_FLIP_DISCARD
            || desc.SwapEffect == Dxgi::DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL;
        let tearing = desc.Flags & Dxgi::DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32 != 0;
        let present_flags = if sync_interval == 0 && flip && tearing {
            Dxgi::DXGI_PRESENT_ALLOW_TEARING
        } else {
            Dxgi::DXGI_PRESENT::default()
        };

        let back_buffer: d3d11::ID3D11Texture2D = unsafe { raw.GetBuffer(0) }.map_err(|err| {
            log::error!("Direct3D 11: GetBuffer(0): {err}");
            crate::SurfaceError::Other("IDXGISwapChain::GetBuffer")
        })?;
        let address = back_buffer.cast::<d3d11::ID3D11Resource>().map_or(0, |r| r.as_raw() as usize);
        if device.shared.buffer_back.swap(address, core::sync::atomic::Ordering::AcqRel) != address {
            device.shared.views_back.lock().clear();
        }

        let in_more = usages_in_more(config);
        if let Err(err) = unsafe {
            raw.SetPrivateData(&GUID_USAGES_IN_MORE, size_of_val(&in_more) as u32, <*const _>::cast(&in_more))
        } {
            log::warn!("Direct3D 11: swap chain SetPrivateData: {err}");
        }

        *chain = Some(SwapChain {
            raw,
            back_buffer,
            format: config.format,
            size: config.extent,
            sync_interval,
            present_flags,
        });
        Ok(())
    }

    unsafe fn unconfigure(&self, device: &Device) {
        if self.swap_chain.write().take().is_some() {
            unsafe { unbind_targets(device) };
        }
    }

    unsafe fn acquire_texture(
        &self,
        _timeout: Option<Duration>,
        _fence: &Fence,
    ) -> Result<crate::AcquiredSurfaceTexture<Api>, crate::SurfaceError> {
        let chain = self.swap_chain.read();
        let sc = chain.as_ref().ok_or(crate::SurfaceError::Other("surface not configured"))?;
        // In FLIP_DISCARD (and in DISCARD), buffer 0 always designates the current back buffer:
        // the one kept since configure works (the clone is only an AddRef).
        let raw: d3d11::ID3D11Resource = sc.back_buffer.cast().map_err(|err| {
            log::error!("Direct3D 11: back buffer to ID3D11Resource: {err}");
            crate::SurfaceError::Other("back buffer")
        })?;
        let mut in_more = 0u32;
        let mut size_value = size_of_val(&in_more) as u32;
        if unsafe { sc.raw.GetPrivateData(&GUID_USAGES_IN_MORE, &mut size_value, <*mut _>::cast(&mut in_more)) }.is_err() {
            in_more = 0;
        }
        let usage = wgt::TextureUses::COLOR_TARGET
            | wgt::TextureUses::COPY_DST
            | (wgt::TextureUses::from_bits_truncate(in_more as _) & wgt::TextureUses::COPY_SRC);
        Ok(crate::AcquiredSurfaceTexture {
            texture: SurfaceTexture {
                texture: Texture {
                    raw,
                    format: sc.format,
                    dimension: wgt::TextureDimension::D2,
                    size: wgt::Extent3d { width: sc.size.width, height: sc.size.height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    usage,
                },
            },
            suboptimal: false,
        })
    }

    unsafe fn discard_texture(&self, texture: SurfaceTexture) {}
}

/// Shows the image (called by `Queue::present`, context already flushed by the submission).
pub(super) unsafe fn present_value(surface: &Surface, texture: SurfaceTexture) -> Result<(), crate::SurfaceError> {
    // The reference to the rendered image is released before Present.
    drop(texture);
    let chain = surface.swap_chain.read();
    let sc = chain.as_ref().ok_or(crate::SurfaceError::Other("surface not configured"))?;
    let hr = unsafe { sc.raw.Present(sc.sync_interval, sc.present_flags) };
    if hr == Dxgi::DXGI_ERROR_DEVICE_REMOVED || hr == Dxgi::DXGI_ERROR_DEVICE_RESET || hr == Dxgi::DXGI_ERROR_DEVICE_HUNG {
        log::error!("Direct3D 11: Present: device lost ({hr:?})");
        return Err(crate::SurfaceError::Lost);
    }
    // DXGI_STATUS_OCCLUDED (window hidden or minimized) is a success: nothing to do.
    if hr.is_err() {
        log::error!("Direct3D 11: Present: {hr:?}");
        return Err(crate::SurfaceError::Other("IDXGISwapChain::Present"));
    }
    Ok(())
}

