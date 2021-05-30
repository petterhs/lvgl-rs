use crate::functions::CoreError;
use crate::Box;
use crate::{disp_drv_register, disp_get_default, get_str_act};
use crate::{Color, Obj};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::NonNull;
use core::{ptr, result};
use embedded_graphics::drawable;
use embedded_graphics::prelude::*;

// TODO: Make this an external configuration
const REFRESH_BUFFER_LEN: usize = 2;
// Declare a buffer for the refresh rate
pub(crate) const BUF_SIZE: usize = lvgl_sys::LV_HOR_RES_MAX as usize * REFRESH_BUFFER_LEN;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DisplayError {
    NotAvailable,
    FailedToRegister,
    NotRegistered,
}

type Result<T> = result::Result<T, DisplayError>;

#[derive(Copy, Clone)]
pub struct Display {
    pub(crate) disp: NonNull<lvgl_sys::lv_disp_t>,
}

impl Display {
    pub(crate) fn from_raw(disp: NonNull<lvgl_sys::lv_disp_t>) -> Self {
        Self { disp }
    }

    pub fn register<C>(embedded_graphics_display: impl DrawTarget<C>) -> Result<Self>
    where
        C: PixelColor + From<Color>,
    {
        let mut display_diver = DisplayDriver::new(DisplayBuffer::new(), embedded_graphics_display);
        Ok(disp_drv_register(&mut display_diver)?)
    }

    pub fn get_str_act(&self) -> Result<Obj> {
        Ok(get_str_act(Some(&self))?)
    }
}

impl Default for Display {
    fn default() -> Self {
        disp_get_default().expect("LVGL must be initialized")
    }
}

#[derive(Copy, Clone)]
pub struct DefaultDisplay {}

impl DefaultDisplay {
    /// Gets the screen active of the default display.
    pub fn get_scr_act() -> Result<Obj> {
        Ok(get_str_act(None)?)
    }
}

#[derive(Copy, Clone)]
pub struct DisplayBuffer {
    disp_buf: lvgl_sys::lv_disp_buf_t,
}

impl DisplayBuffer {
    pub fn new() -> Self {
        let disp_buf = unsafe {
            let mut disp_buf = MaybeUninit::uninit();
            let refresh_buffer = Box::new([lvgl_sys::lv_color_t::default(); BUF_SIZE]);

            lvgl_sys::lv_disp_buf_init(
                disp_buf.as_mut_ptr(),
                Box::into_raw(refresh_buffer) as *mut cty::c_void,
                ptr::null_mut(),
                lvgl_sys::LV_HOR_RES_MAX * REFRESH_BUFFER_LEN as u32,
            );
            disp_buf.assume_init()
        };

        Self { disp_buf }
    }
}

#[derive(Copy, Clone)]
pub struct DisplayDriver<T, C>
where
    T: DrawTarget<C>,
    C: PixelColor + From<Color>,
{
    pub(crate) disp_drv: lvgl_sys::lv_disp_drv_t,
    phantom_display: PhantomData<T>,
    phantom_color: PhantomData<C>,
}

impl<T, C> DisplayDriver<T, C>
where
    T: DrawTarget<C>,
    C: PixelColor + From<Color>,
{
    pub fn new(display_buffer: DisplayBuffer, native_display: T) -> Self {
        let mut disp_drv = unsafe {
            let mut disp_drv = MaybeUninit::uninit();
            lvgl_sys::lv_disp_drv_init(disp_drv.as_mut_ptr());
            disp_drv.assume_init()
        };

        // We need to add to a `Box`, so it's copied to a memory location in the "heap" (LVGL statically allocated heap).
        let disp_buf = Box::new(display_buffer.disp_buf);
        disp_drv.buffer = Box::into_raw(disp_buf) as *mut lvgl_sys::lv_disp_buf_t;

        let native_display = Box::new(DisplayUserData {
            display: native_display,
            phantom: PhantomData,
        });
        disp_drv.user_data =
            Box::into_raw(native_display) as *mut _ as lvgl_sys::lv_disp_drv_user_data_t;

        disp_drv.flush_cb = Some(disp_flush_trampoline::<T, C>);

        // We do not store any memory that can be accidentally deallocated by on the Rust side.
        Self {
            disp_drv,
            phantom_color: PhantomData,
            phantom_display: PhantomData,
        }
    }
}

pub(crate) struct DisplayUserData<T, C>
where
    T: DrawTarget<C>,
    C: PixelColor + From<Color>,
{
    display: T,
    phantom: PhantomData<C>,
}

unsafe extern "C" fn disp_flush_trampoline<T, C>(
    disp_drv: *mut lvgl_sys::lv_disp_drv_t,
    area: *const lvgl_sys::lv_area_t,
    color_p: *mut lvgl_sys::lv_color_t,
) where
    T: DrawTarget<C>,
    C: PixelColor + From<Color>,
{
    let display_driver = *disp_drv;
    if !display_driver.user_data.is_null() {
        let user_data = &mut *(display_driver.user_data as *mut DisplayUserData<T, C>);
        let x1 = (*area).x1;
        let x2 = (*area).x2;
        let y1 = (*area).y1;
        let y2 = (*area).y2;

        let ys = y1..=y2;
        let xs = (x1..=x2).enumerate();
        let x_len = (x2 - x1 + 1) as usize;

        // We use iterators here to ensure that the Rust compiler can apply all possible
        // optimizations at compile time.
        let pixels = ys
            .enumerate()
            .map(|(iy, y)| {
                xs.clone().map(move |(ix, x)| {
                    let color_len = x_len * iy + ix;
                    let lv_color = unsafe { *color_p.add(color_len) };
                    let raw_color = Color::from_raw(lv_color);
                    drawable::Pixel::<C>(Point::new(x as i32, y as i32), raw_color.into())
                })
            })
            .flatten();

        let _ = user_data.display.draw_iter(pixels);
    }
}

impl From<CoreError> for DisplayError {
    fn from(err: CoreError) -> Self {
        use DisplayError::*;
        match err {
            CoreError::ResourceNotAvailable => NotAvailable,
            CoreError::OperationFailed => NotAvailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests;

    #[test]
    fn get_scr_act_return_display() {
        tests::initialize_test();
        let _screen = get_str_act(None).expect("We can get the active screen");
    }

    #[test]
    fn get_default_display() {
        tests::initialize_test();
        let display = Display::default();

        let _screen_direct = display
            .get_str_act()
            .expect("Return screen directly from the display instance");

        let _screen_default =
            DefaultDisplay::get_scr_act().expect("Return screen from the default display");
    }

    #[test]
    fn register_display_directly() -> Result<()> {
        tests::initialize_test();
        let display = Display::default();

        let _screen = display
            .get_str_act()
            .expect("Return screen directly from the display instance");

        Ok(())
    }
}
