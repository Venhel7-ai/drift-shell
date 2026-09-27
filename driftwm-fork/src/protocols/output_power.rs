use std::{collections::HashMap, mem};

use smithay::{
    output::{Output, WeakOutput},
    reexports::{
        wayland_protocols_wlr::output_power_management::v1::server::{
            zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
            zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
            backend::{ClientId, GlobalId},
        },
    },
};

pub trait OutputPowerHandler {
    fn output_power_state(&mut self) -> &mut OutputPowerState;
    fn get_dpms(&mut self, output: &Output) -> Option<bool>;
    fn set_dpms(&mut self, output: &Output, on: bool);
}

#[derive(Debug)]
pub struct OutputPowerState {
    #[allow(dead_code)]
    global: GlobalId,
    output_powers: HashMap<ZwlrOutputPowerV1, zwlr_output_power_v1::Mode>,
}

impl OutputPowerState {
    pub fn new<D, F>(dh: &DisplayHandle, client_filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData> + 'static,
        F: for<'a> Fn(&'a Client) -> bool + Send + Sync + 'static,
    {
        let global = dh.create_global::<D, ZwlrOutputPowerManagerV1, _>(
            1,
            OutputPowerManagerGlobalData {
                filter: Box::new(client_filter),
            },
        );
        Self {
            global,
            output_powers: HashMap::new(),
        }
    }

    /// Re-broadcast `mode` events for any output whose DPMS state changed since
    /// the last refresh. Call this after applying compositor-initiated DPMS
    /// transitions (e.g. drained pending requests). Client-initiated changes are
    /// already broadcast inline.
    pub fn refresh<D: OutputPowerHandler>(state: &mut D) {
        let mut output_powers = mem::take(&mut state.output_power_state().output_powers);
        for (output_power, old_mode) in output_powers.iter_mut() {
            let data = output_power.data::<OutputPowerData>().unwrap();
            if let Some(output) = data.output.upgrade()
                && let Some(on) = state.get_dpms(&output)
            {
                let mode = mode_for(on);
                if mode != *old_mode {
                    output_power.mode(mode);
                    *old_mode = mode;
                }
            }
        }
        state.output_power_state().output_powers = output_powers;
    }
}

pub struct OutputPowerManagerGlobalData {
    filter: Box<dyn for<'a> Fn(&'a Client) -> bool + Send + Sync>,
}

pub struct OutputPowerData {
    output: WeakOutput,
}

impl<D> GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData, D>
    for OutputPowerState
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData>
        + Dispatch<ZwlrOutputPowerManagerV1, ()>
        + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _global_data: &OutputPowerManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &OutputPowerManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrOutputPowerManagerV1, (), D> for OutputPowerState
where
    D: Dispatch<ZwlrOutputPowerManagerV1, ()>
        + Dispatch<ZwlrOutputPowerV1, OutputPowerData>
        + OutputPowerHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _obj: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                let output = Output::from_resource(&output);
                let output_power = data_init.init(
                    id,
                    OutputPowerData {
                        output: output.as_ref().map(|o| o.downgrade()).unwrap_or_default(),
                    },
                );
                if let Some(on) = output.as_ref().and_then(|o| state.get_dpms(o)) {
                    let mode = mode_for(on);
                    output_power.mode(mode);
                    state
                        .output_power_state()
                        .output_powers
                        .insert(output_power, mode);
                } else {
                    output_power.failed();
                }
            }
            zwlr_output_power_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl<D> Dispatch<ZwlrOutputPowerV1, OutputPowerData, D> for OutputPowerState
where
    D: Dispatch<ZwlrOutputPowerV1, OutputPowerData> + OutputPowerHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        data: &OutputPowerData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => {
                let Some(output) = data.output.upgrade() else {
                    obj.failed();
                    state.output_power_state().output_powers.remove(obj);
                    return;
                };
                let on = match mode {
                    WEnum::Value(zwlr_output_power_v1::Mode::On) => true,
                    WEnum::Value(zwlr_output_power_v1::Mode::Off) => false,
                    _ => return,
                };
                state.set_dpms(&output, on);
                if let Some(on) = state.get_dpms(&output) {
                    let mode = mode_for(on);
                    for (output_power, old_mode) in
                        state.output_power_state().output_powers.iter_mut()
                    {
                        let data = output_power.data::<OutputPowerData>().unwrap();
                        if data.output == output && mode != *old_mode {
                            output_power.mode(mode);
                            *old_mode = mode;
                        }
                    }
                } else {
                    obj.failed();
                    state.output_power_state().output_powers.remove(obj);
                }
            }
            zwlr_output_power_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        obj: &ZwlrOutputPowerV1,
        _data: &OutputPowerData,
    ) {
        state.output_power_state().output_powers.remove(obj);
    }
}

fn mode_for(on: bool) -> zwlr_output_power_v1::Mode {
    if on {
        zwlr_output_power_v1::Mode::On
    } else {
        zwlr_output_power_v1::Mode::Off
    }
}

#[macro_export]
macro_rules! delegate_output_power {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: $crate::protocols::output_power::OutputPowerManagerGlobalData
        ] => $crate::protocols::output_power::OutputPowerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: ()
        ] => $crate::protocols::output_power::OutputPowerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::ZwlrOutputPowerV1: $crate::protocols::output_power::OutputPowerData
        ] => $crate::protocols::output_power::OutputPowerState);
    };
}
