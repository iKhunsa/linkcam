//! Vendor extension-unit access through the capture filter's `IKsControl`.
//!
//! Each XU control is addressed by (node id, XU GUID, selector). Node ids
//! come from `IKsTopologyInfo`; the GUIDs are the ones documented by
//! linkcam / the Linux controller and are verified by the `probe` tool.

use linkcam_core::{CameraError, Result};
use std::mem::size_of;
use windows::core::GUID;
use windows::Win32::Media::DirectShow::IBaseFilter;
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsTopologyInfo, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSNODETYPE_DEV_SPECIFIC,
    KSPROPERTY_DESCRIPTION, KSPROPERTY_MEMBERSHEADER, KSPROPERTY_TYPE_BASICSUPPORT, KSPROPERTY_TYPE_GET,
    KSPROPERTY_TYPE_SET, KSPROPERTY_TYPE_TOPOLOGY, KSP_NODE,
};

/// GUID of a UVC extension unit (the property set of its controls).
pub type XuGuid = GUID;

/// Extension-unit GUIDs shared by Link, Link 2 and (assumed) Link 2C.
pub const KNOWN_XU_GUIDS: [XuGuid; 3] = [
    GUID::from_u128(0xfaf1672d_b71b_4793_8c91_7b1c9b7f95f8),
    GUID::from_u128(0xe307e649_4618_a3ff_82fc_2d8b5f216773),
    GUID::from_u128(0xa8bd5df2_1a98_474e_8dd0_d92672d194fa),
];

/// KSPROPERTY_DESCRIPTION.AccessFlags bits.
const ACCESS_GET: u32 = KSPROPERTY_TYPE_GET;
const ACCESS_SET: u32 = KSPROPERTY_TYPE_SET;
/// Largest control payload we read (UVC allows up to 65535, Insta360 uses < 64).
const MAX_CONTROL_LEN: usize = 256;

/// What a control allows, as reported by the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XuAccess {
    pub readable: bool,
    pub writable: bool,
    /// Payload length, when the driver reports one.
    pub length: Option<usize>,
}

pub struct XuClient {
    control: IKsControl,
    topology: IKsTopologyInfo,
}

impl XuClient {
    pub fn new(filter: &IBaseFilter) -> Result<Self> {
        let control: IKsControl = windows::core::Interface::cast(filter)
            .map_err(|e| CameraError::ReadFailed(format!("IKsControl: {e}")))?;
        let topology: IKsTopologyInfo = windows::core::Interface::cast(filter)
            .map_err(|e| CameraError::ReadFailed(format!("IKsTopologyInfo: {e}")))?;
        Ok(Self { control, topology })
    }

    /// Node ids of every vendor-specific (extension unit) node.
    pub fn extension_nodes(&self) -> Vec<u32> {
        let count = unsafe { self.topology.NumNodes() }.unwrap_or(0);
        (0..count)
            .filter(|&id| {
                unsafe { self.topology.get_NodeType(id) }.is_ok_and(|t| t == KSNODETYPE_DEV_SPECIFIC)
            })
            .collect()
    }

    /// Asks the driver whether `node` exposes `selector` of `guid`, and how.
    pub fn access(&self, node: u32, guid: &XuGuid, selector: u8) -> Option<XuAccess> {
        let mut buffer = [0u8; MAX_CONTROL_LEN];
        let returned = self.ks_call(node, guid, selector, KSPROPERTY_TYPE_BASICSUPPORT, &mut buffer).ok()?;
        let description_len = size_of::<KSPROPERTY_DESCRIPTION>();
        if returned < size_of::<u32>() {
            return None;
        }
        let access_flags = u32::from_le_bytes(buffer[..4].try_into().ok()?);
        let length = (returned >= description_len + size_of::<KSPROPERTY_MEMBERSHEADER>()).then(|| {
            let members_size_at = description_len + size_of::<u32>();
            u32::from_le_bytes(buffer[members_size_at..members_size_at + 4].try_into().unwrap_or([0; 4]))
                as usize
        });
        Some(XuAccess {
            readable: access_flags & ACCESS_GET != 0,
            writable: access_flags & ACCESS_SET != 0,
            length: length.filter(|&n| n > 0 && n <= MAX_CONTROL_LEN),
        })
    }

    /// Reads `length` bytes of the control's current value (GET_CUR).
    pub fn get(&self, node: u32, guid: &XuGuid, selector: u8, length: usize) -> Result<Vec<u8>> {
        let mut buffer = vec![0u8; length.min(MAX_CONTROL_LEN)];
        let returned = self.ks_call(node, guid, selector, KSPROPERTY_TYPE_GET, &mut buffer)?;
        buffer.truncate(returned);
        Ok(buffer)
    }

    /// Reads the current value at its real length; the driver reports the size
    /// in the number of bytes returned, so no length has to be known up front.
    pub fn read(&self, node: u32, guid: &XuGuid, selector: u8) -> Result<Vec<u8>> {
        self.get(node, guid, selector, MAX_CONTROL_LEN)
    }

    /// Writes the control's value (SET_CUR). Callers must only pass payloads
    /// observed from the official controller; there is no generic safety net here.
    pub fn set(&self, node: u32, guid: &XuGuid, selector: u8, payload: &[u8]) -> Result<()> {
        let mut buffer = payload.to_vec();
        self.ks_call(node, guid, selector, KSPROPERTY_TYPE_SET, &mut buffer).map(|_| ()).map_err(
            |e| match e {
                CameraError::ReadFailed(why) => CameraError::WriteFailed(why),
                other => other,
            },
        )
    }

    fn ks_call(&self, node: u32, guid: &XuGuid, selector: u8, op: u32, data: &mut [u8]) -> Result<usize> {
        let request = KSP_NODE {
            Property: KSIDENTIFIER {
                Anonymous: KSIDENTIFIER_0 {
                    Anonymous: KSIDENTIFIER_0_0 {
                        Set: *guid,
                        Id: selector as u32,
                        Flags: op | KSPROPERTY_TYPE_TOPOLOGY,
                    },
                },
            },
            NodeId: node,
            Reserved: 0,
        };
        let mut returned = 0u32;
        unsafe {
            self.control.KsProperty(
                &request.Property,
                size_of::<KSP_NODE>() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut returned,
            )
        }
        .map_err(|e| CameraError::ReadFailed(format!("selector 0x{selector:02x} on node {node}: {e}")))?;
        Ok(returned as usize)
    }
}
