//! Graphics plumbing (as in Stash): the Windows compositor for everything on screen,
//! Direct2D/DirectWrite to paint pixels into composition surfaces.

use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct2D::{D2D1CreateFactory, ID2D1DeviceContext, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT};
use windows::core::{Interface, Result};
use windows_numerics::Matrix3x2;

use crate::paint::color;

pub struct Gfx {
    pub compositor: Compositor,
    device: CompositionGraphicsDevice,
    pub dwrite: IDWriteFactory,
    _d3d: ID3D11Device,
    _queue: DispatcherQueueController,
}

fn create_d3d(kind: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
    }
    device.ok_or_else(windows::core::Error::from_win32)
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let queue = CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_STA,
            })?;
            let compositor = Compositor::new()?;
            let d3d = create_d3d(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_d3d(D3D_DRIVER_TYPE_WARP))?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d = factory.CreateDevice(&dxgi)?;
            let device = compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&d2d)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self { compositor, device, dwrite, _d3d: d3d, _queue: queue })
        }
    }

    pub fn target(&self, hwnd: HWND) -> Result<DesktopWindowTarget> {
        unsafe { self.compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, false) }
    }

    /// A new composition surface of `w`×`h` pixels, painted by `draw`.
    pub fn surface(&self, w: f32, h: f32, draw: impl FnOnce(&ID2D1DeviceContext)) -> Result<CompositionDrawingSurface> {
        let surface = self.device.CreateDrawingSurface(
            Size { Width: w.max(1.0).ceil(), Height: h.max(1.0).ceil() },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?;
        unsafe {
            let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
            dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            dc.Clear(Some(&color(0, 0, 0, 0.0)));
            draw(&dc);
            interop.EndDraw()?;
        }
        Ok(surface)
    }
}
