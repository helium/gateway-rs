const RETIRED_NETID: u32 = 0x200010;

type DevAddr = u32;
type SubnetAddr = u32;
type NetID = u32;
type NetClass = u8;

/// Does this LoRaWAN devaddr belong to the Helium network?
/// netid_list contains Helium's ordered list of assigned NetIDs
///
pub fn is_local_devaddr(devaddr: DevAddr, netid_list: &[NetID]) -> bool {
    let netid = parse_netid(devaddr);
    is_local_netid(netid, netid_list)
}

/// Translate from a Helium subnet address to a LoRaWAN devaddr.
/// netid_list contains Helium's ordered list of assigned NetIDs
///
pub fn devaddr_from_subnet(subnetaddr: SubnetAddr, netid_list: &[NetID]) -> Option<DevAddr> {
    let netid = subnet_addr_to_netid(subnetaddr, netid_list);
    if netid.is_some() {
        let (lower, _upper) = netid_addr_range(netid.unwrap(), netid_list);
        Some(devaddr(netid.unwrap(), subnetaddr - lower))
    } else {
        None
    }
}

/// Translate from a LoRaWAN devaddr to a Helium subnet address.
/// netid_list contains Helium's ordered list of assigned NetIDs
///
pub fn subnet_from_devaddr(devaddr: DevAddr, netid_list: &[NetID]) -> Option<SubnetAddr> {
    let netid = parse_netid(devaddr);
    let (lower, _upper) = netid_addr_range(netid, netid_list);
    Some(lower + nwk_addr(devaddr))
}

//
// Internal functions
//
// Note - function and var names correspond closely to the LoRaWAN spec.
//

fn netid_class(netid: NetID) -> NetClass {
    let netclass: NetClass = (netid >> 21) as NetClass;
    netclass
}

fn addr_len(netclass: NetClass) -> u32 {
    *[25, 24, 20, 17, 15, 13, 10, 7]
        .get(netclass as usize)
        .unwrap_or(&0)
}

fn id_len(netclass: NetClass) -> u32 {
    *[6, 6, 9, 11, 12, 13, 15, 17]
        .get(netclass as usize)
        .unwrap_or(&0)
}

fn subnet_addr_to_netid(subnetaddr: SubnetAddr, netid_list: &[NetID]) -> Option<NetID> {
    let netid = *netid_list
        .iter()
        .find(|item| subnet_addr_within_range(subnetaddr, **item, netid_list))
        .unwrap_or(&0);
    if netid == 0 {
        None
    } else {
        Some(netid)
    }
}

fn subnet_addr_within_range(subnetaddr: SubnetAddr, netid: NetID, netid_list: &[NetID]) -> bool {
    let (lower, upper) = netid_addr_range(netid, netid_list);
    (subnetaddr >= lower) && (subnetaddr < upper)
}

fn var_net_class(netclass: NetClass) -> u32 {
    let idlen = id_len(netclass);
    match netclass {
        0 => 0,
        1 => 0b10u32 << idlen,
        2 => 0b110u32 << idlen,
        3 => 0b1110u32 << idlen,
        4 => 0b11110u32 << idlen,
        5 => 0b111110u32 << idlen,
        6 => 0b1111110u32 << idlen,
        7 => 0b11111110u32 << idlen,
        _ => 0,
    }
}

fn var_netid(netclass: NetClass, netid: NetID) -> NetID {
    netid << addr_len(netclass)
}

fn devaddr(netid: NetID, nwkaddr: u32) -> DevAddr {
    let netclass = netid_class(netid);
    let id = netid & 0b111111111111111111111;
    let addr = var_net_class(netclass) | id;
    var_netid(netclass, addr) | nwkaddr
}

fn is_local_netid(netid: NetID, netid_list: &[NetID]) -> bool {
    if netid == RETIRED_NETID {
        true
    } else {
        netid_list.contains(&netid)
    }
}

fn netid_type(devaddr: DevAddr) -> NetClass {
    fn netid_shift_prefix(prefix: u8, index: u8) -> NetClass {
        if (prefix & (1 << index)) == 0 {
            7 - index
        } else if index > 0 {
            netid_shift_prefix(prefix, index - 1)
        } else {
            0
        }
    }

    let n_bytes = devaddr.to_be_bytes();
    let first = n_bytes[0];
    netid_shift_prefix(first, 7)
}

fn parse_netid(devaddr: DevAddr) -> NetID {
    fn get_netid(devaddr: u32, prefix_len: u8, nwkidbits: u32) -> u32 {
        (devaddr << (prefix_len - 1)) >> (31 - nwkidbits)
    }

    let net_type = netid_type(devaddr);
    let id = get_netid(devaddr, net_type + 1, id_len(net_type));
    id | ((net_type as u32) << 21)
}

fn netid_addr_range(netid: NetID, netid_list: &[NetID]) -> (SubnetAddr, SubnetAddr) {
    let mut lower: u32 = 0;
    let mut upper: u32 = 0;
    // 95% of traffic is non-Helium so netid_list.contains will usually be false
    if netid_list.contains(&netid) {
        // 5% code path
        for item in netid_list {
            let size = netid_size(*item);
            if *item == netid {
                upper += size;
                break;
            }
            lower += size;
            upper = lower;
        }
    }
    (lower, upper)
}

fn nwk_addr(devaddr: DevAddr) -> u32 {
    let netid = parse_netid(devaddr);
    let len = addr_len(netid_class(netid));
    let mask = (1 << len) - 1;
    devaddr & mask
}

fn netid_size(netid: NetID) -> u32 {
    1 << addr_len(netid_class(netid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr_bit_len(devaddr: u32) -> u32 {
        let netid = parse_netid(devaddr);
        addr_len(netid_class(netid))
    }

    #[allow(non_snake_case)]
    #[test]
    fn test_netid() {
        // LegacyDevAddr = <<$H:7, 0:25>>,
        let LegacyNetID: NetID = RETIRED_NETID;

        let NetID00: NetID = 0xE00001;
        let NetID01: NetID = 0xC00035;
        let NetID02: NetID = 0x60002D;
        let NetIDExt: NetID = 0xC00050;

        // Class 6
        let DevAddr00: DevAddr = 0x90000000;
        let DevAddr01: DevAddr = 0xFC00D410;
        let DevAddr02: DevAddr = 0xE05A0008;

        let NetWidth0 = addr_len(netid_class(NetID00));
        assert_eq!(7, NetWidth0);
        let NetWidth1 = addr_len(netid_class(NetID01));
        assert_eq!(10, NetWidth1);
        let NetWidth2 = addr_len(netid_class(NetID02));
        assert_eq!(17, NetWidth2);
        let NetSize0 = netid_size(NetID00);
        assert_eq!(128, NetSize0);
        let NetSize1 = netid_size(NetID01);
        assert_eq!(1024, NetSize1);
        let NetSize2 = netid_size(NetID02);
        assert_eq!(131072, NetSize2);

        let NetIDList: Vec<NetID> = vec![NetID00, NetID01, NetID02];
        let LocalTrue = is_local_netid(NetID01, &NetIDList);
        let LocalFalse = is_local_netid(NetIDExt, &NetIDList);
        let LegacyLocal = is_local_netid(LegacyNetID, &NetIDList);
        assert_eq!(true, LocalTrue);
        assert_eq!(false, LocalFalse);
        assert_eq!(true, LegacyLocal);

        let DevAddrLegacy = devaddr(LegacyNetID, 0);
        assert_eq!(DevAddr00, DevAddrLegacy);
        let DevAddr1 = devaddr(NetID01, 16);
        assert_eq!(DevAddr01, DevAddr1);
        let DevAddr2 = devaddr(NetID02, 8);
        assert_eq!(DevAddr02, DevAddr2);

        let NetIDType00 = netid_type(DevAddr00);
        assert_eq!(1, NetIDType00);
        let NetIDType01 = netid_type(DevAddr01);
        assert_eq!(6, NetIDType01);
        let NetIDType02 = netid_type(DevAddr02);
        assert_eq!(3, NetIDType02);

        let NetIDType0 = netid_type(DevAddrLegacy);
        assert_eq!(1, NetIDType0);
        let NetIDType1 = netid_type(DevAddr1);
        assert_eq!(6, NetIDType1);
        let NetIDType2 = netid_type(DevAddr2);
        assert_eq!(3, NetIDType2);

        let NetIDType0 = netid_type(DevAddrLegacy);
        assert_eq!(1, NetIDType0);
        let NetIDType1 = netid_type(DevAddr1);
        assert_eq!(6, NetIDType1);
        let NetIDType2 = netid_type(DevAddr2);
        assert_eq!(3, NetIDType2);

        let NetID_0 = parse_netid(DevAddr00);
        assert_eq!(NetID_0, LegacyNetID);
        let NetID_1_a = parse_netid(0xFC00D410);
        assert_eq!(NetID_1_a, 0xC00035);
        let NetID_1 = parse_netid(DevAddr01);
        assert_eq!(NetID_1, NetID01);
        let NetID_2 = parse_netid(DevAddr02);
        assert_eq!(NetID_2, NetID02);

        let NetID0 = parse_netid(DevAddrLegacy);
        assert_eq!(NetID0, LegacyNetID);
        let NetID1 = parse_netid(DevAddr1);
        assert_eq!(NetID1, NetID01);
        let NetID2 = parse_netid(DevAddr2);
        assert_eq!(NetID2, NetID02);

        let Width_0 = addr_bit_len(DevAddr00);
        assert_eq!(24, Width_0);
        let Width_1 = addr_bit_len(DevAddr01);
        assert_eq!(10, Width_1);
        let Width_2 = addr_bit_len(DevAddr02);
        assert_eq!(17, Width_2);

        let Width0 = addr_bit_len(DevAddrLegacy);
        assert_eq!(24, Width0);
        let Width1 = addr_bit_len(DevAddr1);
        assert_eq!(10, Width1);
        let Width2 = addr_bit_len(DevAddr2);
        assert_eq!(17, Width2);

        let NwkAddr0 = nwk_addr(DevAddr00);
        assert_eq!(0, NwkAddr0);
        let NwkAddr1 = nwk_addr(DevAddr01);
        assert_eq!(16, NwkAddr1);
        let NwkAddr2 = nwk_addr(DevAddr02);
        assert_eq!(8, NwkAddr2);

        // Backwards DevAddr compatibility test
        // DevAddr00 is a legacy Helium Devaddr.  The NetID is retired.
        // By design we do compute a proper subnet (giving us a correct OUI route),
        // but if we compute the associated DevAddr for this subnet (for the Join request)
        // we'll get a new one associated with a current and proper NetID
        // In other words, DevAddr00 is not equal to DevAddr000.
        let Subnet0 = subnet_from_devaddr(DevAddr00, &NetIDList);
        assert_eq!(Some(0), Subnet0);
        let DevAddr000 = devaddr_from_subnet(Subnet0.unwrap(), &NetIDList);
        // By design the reverse DevAddr will have a correct NetID
        assert_ne!(DevAddr000.unwrap(), DevAddr00);
        assert_eq!(Some(0xFE000080), DevAddr000);
        let DevAddr000NetID = parse_netid(DevAddr000.unwrap());
        assert_eq!(NetID00, DevAddr000NetID);

        let Subnet1 = subnet_from_devaddr(DevAddr01, &NetIDList);
        assert_eq!((1 << 7) + 16, Subnet1.unwrap());
        let DevAddr001 = devaddr_from_subnet(Subnet1.unwrap(), &NetIDList);
        assert_eq!(DevAddr001.unwrap(), DevAddr01);

        let Subnet1 = subnet_from_devaddr(DevAddr01, &NetIDList);
        assert_eq!((1 << 7) + 16, Subnet1.unwrap());
        let DevAddr001 = devaddr_from_subnet(Subnet1.unwrap(), &NetIDList);
        assert_eq!(DevAddr001.unwrap(), DevAddr01);

        let Subnet2 = subnet_from_devaddr(DevAddr02, &NetIDList);
        assert_eq!((1 << 7) + (1 << 10) + 8, Subnet2.unwrap());
        let DevAddr002 = devaddr_from_subnet(Subnet2.unwrap(), &NetIDList);
        assert_eq!(DevAddr002.unwrap(), DevAddr02);
    }

    #[test]
    fn test_id() {
        // CP data (matches Erlang test cases)
        // <<91, 255, 255, 255>> "[45] == 2D == 45 type 0"
        assert_eq!(0x00002D, parse_netid(0x5BFFFFFF));
        // <<173, 255, 255, 255>> "[45] == 2D == 45 type 1"
        assert_eq!(0x20002D, parse_netid(0xADFFFFFF));
        // <<214, 223, 255, 255>> "[1,109] == 16D == 365 type 2"
        assert_eq!(0x40016D, parse_netid(0xD6DFFFFF));
        // <<235, 111, 255, 255>>), "[5,183] == 5B7 == 1463 type 3"
        assert_eq!(0x6005B7, parse_netid(0xEB6FFFFF));
        // <<245, 182, 255, 255>>), "[11, 109] == B6D == 2925 type 4"
        assert_eq!(0x800B6D, parse_netid(0xF5B6FFFF));
        // println!(
        //     "left: {:#04X?} right: {:#04X?}",
        //     0xA016DB,
        //     parse_netid(0xFADB7FFF)
        // );
        // <<250, 219, 127, 255>>), "[22,219] == 16DB == 5851 type 5"
        assert_eq!(0xA016DB, parse_netid(0xFADB7FFF));
        // <<253, 109, 183, 255>> "[91, 109] == 5B6D == 23405 type 6"
        assert_eq!(0xC05B6D, parse_netid(0xFD6DB7FF));
        // <<254, 182, 219, 127>> "[1,109,182] == 16DB6 == 93622 type 7"
        assert_eq!(0xE16DB6, parse_netid(0xFEB6DB7F));
        println!(
            "left: {:#04X?} right: {:#04X?}",
            0xA016DB,
            parse_netid(0xFFFFFFFF)
        );
        // FixME - Invalid NetID type
        assert_eq!(127, parse_netid(0xFFFFFFFF));

        // Actility spreadsheet examples
        assert_eq!(0, parse_netid(0));
        assert_eq!(1, parse_netid(1 << 25));
        assert_eq!(2, parse_netid(1 << 26));

        // Mis-parsed as netid 4 of type 3
        assert_eq!(0x600004, parse_netid(0xE009ABCD));
        // Valid DevAddr, NetID not assigned
        assert_eq!(0x20002D, parse_netid(0xADFFFFFF));
        // Less than 32 bit number
        assert_eq!(0, parse_netid(46377));

        // Louis test data
        assert_eq!(0x600002, parse_netid(0xE0040001));
        assert_eq!(0x600002, parse_netid(0xE0052784));
        assert_eq!(0x000002, parse_netid(0x0410BEA3));
    }
}
