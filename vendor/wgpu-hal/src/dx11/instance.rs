//! Instance (DXGI factory) and list of adapters.
//!
//! `enumerate_adapters`: walk the DXGI adapters (`self.factory.enumerate_adapters()`
//! of `auxil::dxgi::factory`), try D3D11CreateDevice (level 11_1 then 11_0) on each one,
//! discard those below 11_0 or without `ConstantBufferOffsetting` and `ConstantBufferPartialUpdate`
//! (D3D11_FEATURE_D3D11_OPTIONS), and return one `ExposedAdapter` per adapter kept, with
//! `adapter::capabilities_list(...)` (features, capabilities) and the `AdapterInfo` (name, vendor, driver).

use alloc::{format, string::String, vec::Vec};

use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        Graphics::Dxgi,
    },
};

use super::{adapter, Adapter, Api, Instance, Surface};

/// Microsoft Basic Render Driver (WARP), software adapter.
const VENDOR_MICROSOFT: u32 = 0x1414;
const DEVICE_BASIC_RENDER: u32 = 0x8c;
/// Below this amount of dedicated memory, the adapter is considered integrated.
const DEDICATED_MEMORY_BUILTIN: usize = 512 << 20;

/// A DXGI adapter turned into a wgpu adapter, or `None` if it does not suit Direct3D 11.
fn expose(adapter_value: crate::auxil::dxgi::factory::DxgiAdapter, flags: wgt::InstanceFlags) -> Option<crate::ExposedAdapter<Api>> {
    let raw = adapter_value.cast::<Dxgi::IDXGIAdapter1>().ok()?;
    let desc = unsafe { raw.GetDesc1() }.ok()?;
    let name_value = crate::auxil::dxgi::conv::map_adapter_name(desc.Description);

    let software = Dxgi::DXGI_ADAPTER_FLAG(desc.Flags as i32).contains(Dxgi::DXGI_ADAPTER_FLAG_SOFTWARE)
        || (desc.VendorId == VENDOR_MICROSOFT && desc.DeviceId == DEVICE_BASIC_RENDER);
    if software {
        log::info!("Direct3D 11: software adapter skipped ({name_value})");
        return None;
    }

    // Test device: released at the end of this function.
    let (device_value, _context, level_value) = match adapter::create_device(&raw, false) {
        Ok(r) => r,
        Err(e) => {
            log::info!("Direct3D 11: {name_value} skipped, D3D11CreateDevice: {e}");
            return None;
        }
    };
    if !adapter::device_enough(&device_value, level_value) {
        log::info!(
            "Direct3D 11: {name_value} skipped (level {:#x}, or no ID3D11Device1 / constant buffer offsets)",
            level_value.0
        );
        return None;
    }
    let (features, capabilities) = adapter::capabilities_list(&device_value, level_value);

    // Driver version (UMD), in four 16-bit parts.
    let version = unsafe { raw.CheckInterfaceSupport(&Dxgi::IDXGIDevice::IID) }
        .map(|v| {
            let v = v as u64;
            format!("{}.{}.{}.{}", (v >> 48) as u16, (v >> 32) as u16, (v >> 16) as u16, v as u16)
        })
        .unwrap_or_default();
    let device_type = if desc.DedicatedVideoMemory < DEDICATED_MEMORY_BUILTIN {
        wgt::DeviceType::IntegratedGpu
    } else {
        wgt::DeviceType::DiscreteGpu
    };
    let info = wgt::AdapterInfo {
        name: name_value,
        vendor: desc.VendorId,
        device: desc.DeviceId,
        driver: String::from("Direct3D 11"),
        driver_info: version,
        transient_saves_memory: Some(false),
        ..wgt::AdapterInfo::new(device_type, wgt::Backend::Dx12)
    };
    log::info!("Direct3D 11: adapter {} (level {:#x}, driver {})", info.name, level_value.0, info.driver_info);

    Some(crate::ExposedAdapter {
        adapter: Adapter { raw, feature_level: level_value, info: info.clone(), flags },
        info,
        features,
        capabilities,
    })
}

impl crate::Instance for Instance {
    type A = Api;

    unsafe fn init(desc: &crate::InstanceDescriptor<'_>) -> Result<Self, crate::InstanceError> {
        let (lib, factory) = crate::auxil::dxgi::factory::create_factory(desc.flags)?;
        Ok(Instance { factory, _lib_dxgi: lib, flags: desc.flags })
    }

    unsafe fn create_surface(
        &self,
        _display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
    ) -> Result<Surface, crate::InstanceError> {
        match window_handle {
            raw_window_handle::RawWindowHandle::Win32(h) => Ok(Surface {
                factory: self.factory.clone(),
                hwnd: HWND(h.hwnd.get() as *mut _),
                swap_chain: parking_lot::RwLock::new(None),
            }),
            _ => Err(crate::InstanceError::new(String::from("Direct3D 11: Win32 window expected"))),
        }
    }

    unsafe fn enumerate_adapters(&self, _surface_hint: Option<&Surface>) -> Vec<crate::ExposedAdapter<Api>> {
        crate::auxil::dxgi::factory::enumerate_adapters(self.factory.clone())
            .into_iter()
            .filter_map(|adapter_value| expose(adapter_value, self.flags))
            .collect()
    }
}

// Called by wgpu-core under `cfg(dx12)`; unneeded for OpenXIII (Win32 window only).
impl Instance {
    pub unsafe fn create_surface_from_visual(&self, visual: *mut core::ffi::c_void) -> Surface {
        unimplemented!("Direct3D 11: DirectComposition surfaces not supported")
    }
    pub unsafe fn create_surface_from_surface_handle(&self, surface_handle: *mut core::ffi::c_void) -> Surface {
        unimplemented!("Direct3D 11: surface handles not supported")
    }
    pub unsafe fn create_surface_from_swap_chain_panel(&self, swap_chain_panel: *mut core::ffi::c_void) -> Surface {
        unimplemented!("Direct3D 11: SwapChainPanel not supported")
    }
}
