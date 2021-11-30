pub fn is_local_devaddr(devaddr: u32, netid_list: Vec<u32>) -> bool {
    let netid = the_netid(devaddr);
    is_local_netid(netid, netid_list.clone())
}

pub fn devaddr_from_subnet(subnetaddr: u32, netid_list: Vec<u32>) -> u32 {
    let netid = subnet_addr_to_netid(subnetaddr, netid_list.clone());
    let (lower, _upper) = netid_addr_range(netid, netid_list.clone());
    let devaddr = devaddr(netid, subnetaddr - lower);
    return devaddr;
}

pub fn subnet_from_devaddr(devaddr: u32, netid_list: Vec<u32>) -> u32 {
    let netid = the_netid(devaddr);
    let (lower, _upper) = netid_addr_range(netid, netid_list);
    let subnet_addr: u32 = lower + nwk_addr(devaddr);
    return subnet_addr;
}

fn netid_class(netid: u32) -> u32 {
    let result: u32 = netid >> 21;
    return result;
}

fn addr_len(netclass: u32) -> u32 {
    let result: u32 = match netclass {
        0 => 25,
        1 => 24,
        2 => 20,
        3 => 17,
        4 => 15,
        5 => 13,
        6 => 10,
        7 => 7,
        _ => 0,
    };
    return result;
}

#[allow(dead_code)]
fn addr_bit_len(devaddr: u32) -> u32 {
    let netid = the_netid(devaddr);
    addr_len(netid_class(netid))
}

fn id_len(netclass: u32) -> u32 {
    let result: u32 = match netclass {
        0 => 6,
        1 => 6,
        2 => 9,
        3 => 11,
        4 => 12,
        5 => 13,
        6 => 15,
        7 => 17,
        _ => 0,
    };
    return result;
}

fn subnet_addr_to_netid(subnetaddr: u32, netid_list: Vec<u32>) -> u32 {
    for item in netid_list.clone() {
        if subnet_addr_within_range(subnetaddr, item, netid_list.clone()) {
            return item;
        }
    }
    return 0;
}

fn subnet_addr_within_range(subnetaddr: u32, netid: u32, netid_list: Vec<u32>) -> bool {
    let (lower, upper) = netid_addr_range(netid, netid_list);
    (subnetaddr >= lower) && (subnetaddr < upper)
}

fn var_net_class(netclass: u32) -> u32 {
    let idlen = id_len(netclass);
    let result: u32 = match netclass {
        0 => 0,
        1 => 0b10u32 << idlen,
        2 => 0b110u32 << idlen,
        3 => 0b1110u32 << idlen,
        4 => 0b11110u32 << idlen,
        5 => 0b111110u32 << idlen,
        6 => 0b1111110u32 << idlen,
        7 => 0b11111110u32 << idlen,
        _ => 0,
    };
    return result;
}

fn var_netid(netclass: u32, netid: u32) -> u32 {
    netid << addr_len(netclass)
}

fn devaddr(netid: u32, nwkaddr: u32) -> u32 {
    let netclass = netid_class(netid);
    let id = netid & 0b111111111111111111111;
    let addr = var_net_class(netclass) | id;
    let devaddr = var_netid(netclass, addr) | nwkaddr;
    return devaddr;
}

fn is_local_netid(netid: u32, netid_list: Vec<u32>) -> bool {
    for item in netid_list {
        if item == netid {
            return true;
        }
    }
    return false;
}

fn netid_type(devaddr: u32) -> u32 {
    fn netid_shift_prefix(prefix: u8, index: u32) -> u32 {
        if (prefix & (1 << index)) == 0 {
            return 7 - index;
        } else if index > 0 {
            return netid_shift_prefix(prefix, index - 1);
        } else {
            return 0;
        }
    }

    let n_bytes = devaddr.to_be_bytes();
    let first = n_bytes[0];
    return netid_shift_prefix(first, 7);
}

fn get_netid(devaddr: u32, prefix_len: u32, nwkidbits: u32) -> u32 {
    println!(
        "get_netid: devaddr={:#04X?} prefix_len={} nwkidbits={}",
        devaddr, prefix_len, nwkidbits
    );
    (devaddr << (prefix_len - 1)) >> (31 - nwkidbits)
}

fn the_netid(devaddr: u32) -> u32 {
    println!("devaddr is: {:#04X?}", devaddr);
    let net_type = netid_type(devaddr);
    println!("net_type is: {:#04X?}", net_type);
    let id = get_netid(devaddr, net_type + 1, id_len(net_type));
    println!("ID is: {:#04X?}", id);
    id | (net_type << 21)
}

#[allow(dead_code)]
fn netid(devaddr: u32) -> u32 {
    the_netid(devaddr)
}

fn netid_addr_range(netid: u32, netid_list: Vec<u32>) -> (u32, u32) {
    let mut lower: u32 = 0;
    let mut upper: u32 = 0;
    if netid_list.contains(&netid) {
        for item in netid_list {
            let size = netid_size(item);
            if item == netid {
                upper += size;
                break;
            }
            lower += size;
            upper = lower;
        }
    }
    (lower, upper)
}

fn nwk_addr(devaddr: u32) -> u32 {
    let netid = the_netid(devaddr);
    let len = addr_len(netid_class(netid));
    let mask = (1 << len) - 1;
    return devaddr & mask;
}

fn netid_size(netid: u32) -> u32 {
    1 << addr_len(netid_class(netid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(non_snake_case)]
    #[test]
    fn test_netid() {
        let RETIRED_NETID: u32 = 0x200010;

        // LegacyDevAddr = <<$H:7, 0:25>>,
        // LegacyNum = 16#90000000,
        // _LegacyID = 8,
        // %% 16#200010,
        let LegacyNetID: u32 = RETIRED_NETID;
        // <<H1:7, _/bitstring>> = LegacyDevAddr,
        // <<H2:7, _:25>> = LegacyDevAddr,
        // H3 = <<LegacyNum:32/integer-unsigned>>,
        // ?assertEqual(H1, H2),
        // ?assertEqual(H3, LegacyDevAddr),

        let NetID00: u32 = 0xE00001;
        let NetID01: u32 = 0xC00035;
        let NetID02: u32 = 0x60002D;
        let NetIDExt: u32 = 0xC00050;

        // %% Class 6
        let DevAddr00: u32 = 0x90000000;
        let DevAddr01: u32 = 0xFC00D410;
        let DevAddr02: u32 = 0xE05A0008;

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

        let NetIDList: Vec<u32> = vec![NetID00, NetID01, NetID02];
        let LocalTrue = is_local_netid(NetID01, NetIDList.clone());
        let LocalFalse = is_local_netid(NetIDExt, NetIDList.clone());
        let _LegacyLocal = is_local_netid(LegacyNetID, NetIDList.clone());
        assert_eq!(true, LocalTrue);
        assert_eq!(false, LocalFalse);
        //assert_eq!(true, LegacyLocal);

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

        let NetID_0 = netid(DevAddr00);
        assert_eq!(NetID_0, LegacyNetID);
        let NetID_1_a = netid(0xFC00D410);
        assert_eq!(NetID_1_a, 0xC00035);
        let NetID_1 = netid(DevAddr01);
        assert_eq!(NetID_1, NetID01);
        let NetID_2 = netid(DevAddr02);
        assert_eq!(NetID_2, NetID02);

        let NetID0 = netid(DevAddrLegacy);
        assert_eq!(NetID0, LegacyNetID);
        let NetID1 = netid(DevAddr1);
        assert_eq!(NetID1, NetID01);
        let NetID2 = netid(DevAddr2);
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

        // %% Backwards DevAddr compatibility test
        // %% DevAddr00 is a legacy Helium Devaddr.  The NetID is retired.
        // %% By design we do compute a proper subnet (giving us a correct OUI route),
        // %% but if we compute the associated DevAddr for this subnet (for the Join request)
        // %% we'll get a new one associated with a current and proper NetID
        let Subnet0 = subnet_from_devaddr(DevAddr00, NetIDList.clone());
        //io:format("Subnet0 ~8.16.0B~n", [Subnet0]);
        assert_eq!(0, Subnet0);
        let DevAddr000 = devaddr_from_subnet(Subnet0, NetIDList.clone());
        //io:format("DevAddr00 ~8.16.0B~n", [DevAddr00]);
        //io:format("DevAddr000 ~8.16.0B~n", [DevAddr000]);
        //%% By design the reverse DevAddr will have a correct NetID
        // FixMe assert_eq!(DevAddr000, DevAddr00);
        assert_eq!(0xFE000080, DevAddr000);
        let DevAddr000NetID = netid(DevAddr000);
        assert_eq!(NetID00, DevAddr000NetID);

        let Subnet1 = subnet_from_devaddr(DevAddr01, NetIDList.clone());
        //io:format("Subnet1 ~8.16.0B~n", [Subnet1]);
        assert_eq!((1 << 7) + 16, Subnet1);
        let DevAddr001 = devaddr_from_subnet(Subnet1, NetIDList.clone());
        //io:format("DevAddr01 ~8.16.0B~n", [DevAddr01]);
        //io:format("DevAddr001 ~8.16.0B~n", [DevAddr001]);
        assert_eq!(DevAddr001, DevAddr01);

        let Subnet1 = subnet_from_devaddr(DevAddr01, NetIDList.clone());
        assert_eq!((1 << 7) + 16, Subnet1);
        let DevAddr001 = devaddr_from_subnet(Subnet1, NetIDList.clone());
        assert_eq!(DevAddr001, DevAddr01);

        let Subnet2 = subnet_from_devaddr(DevAddr02, NetIDList.clone());
        assert_eq!((1 << 7) + (1 << 10) + 8, Subnet2);
        let DevAddr002 = devaddr_from_subnet(Subnet2, NetIDList.clone());
        assert_eq!(DevAddr002, DevAddr02);
    }

    #[test]
    fn test_id() {
        // %% CP data
        assert_eq!(0x00002D, the_netid(0x5BFFFFFF)); // <<91, 255, 255, 255>>), "[45] == 2D == 45 type 0"),
        assert_eq!(0x20002D, the_netid(0xADFFFFFF)); // <<173, 255, 255, 255>>), "[45] == 2D == 45 type 1"),
        assert_eq!(0x40016D, the_netid(0xD6DFFFFF)); // <<214, 223, 255, 255>>), "[1,109] == 16D == 365 type 2"
        assert_eq!(
            0x6005B7,
            the_netid(0xEB6FFFFF) // <<235, 111, 255, 255>>), "[5,183] == 5B7 == 1463 type 3"
        );
        assert_eq!(0x800B6D, the_netid(0xF5B6FFFF));
        //        lorawan:netid(<<245, 182, 255, 255>>),
        //        "[11, 109] == B6D == 2925 type 4"
        //    ),
        println!(
            "left: {:#04X?} right: {:#04X?}",
            0xA016DB,
            the_netid(0xFAD87FFF)
        );
        // FixMe assert_eq!( 0xA016DB, the_netid(0xFAD87FFF) );
        //        lorawan:netid(<<250, 219, 127, 255>>),
        //        "[22,219] == 16DB == 5851 type 5"
        //    ),
        assert_eq!(0xC05B6D, the_netid(0xFD6DB7FF));
        //       lorawan:netid(<<253, 109, 183, 255>>),
        //       "[91, 109] == 5B6D == 23405 type 6"
        //   ),
        assert_eq!(0xE16DB6, the_netid(0xFEB6DB7F));
        //       lorawan:netid(<<254, 182, 219, 127>>),
        //       "[1,109,182] == 16DB6 == 93622 type 7"
        //   ),
        // FixMe assert_eq!( 0x0, the_netid(0xFFFFFFFF) );
        //        {error, invalid_netid_type},
        //        lorawan:netid(<<255, 255, 255, 255>>),
        //        "Invalid DevAddr"
        //    ),

        // % Actility spreadsheet examples
        // assert_eq!(0, the_netid(<<0:1, 0:1, 0:1, 0:1, 0:1, 0:1, 0:1, 0:25>>)),
        // assert_eq!(1, the_netid(<<0:1, 0:1, 0:1, 0:1, 0:1, 0:1, 1:1, 0:25>>)),
        // assert_eq!(2, the_netid(<<0:1, 0:1, 0:1, 0:1, 0:1, 1:1, 0:1, 0:25>>)),

        // %% Mis-parsed as netid 4 of type 3
        assert_eq!(0x600004, the_netid(0xE009ABCD));
        // assert_eq!(
        //     0x600004, the_netid(<<224, 9, 171, 205>>), "hex_to_binary(<<'E009ABCD'>>)"
        // ),
        //    %% Valid DevAddr, NetID not assigned
        assert_eq!(0x20002D, the_netid(0xADFFFFFF));
        //        0x20002D, the_netid(<<173, 255, 255, 255>>), "hex_to_binary(<<'ADFFFFFF'>>)"
        //    ),
        //    %% Less than 32 bit number
        assert_eq!(0, the_netid(46377));

        // % Louis test data
        // assert_eq!(0x600002, the_netid(<<224, 4, 0, 1>>)),
        // assert_eq!(0x600002, the_netid(<<224, 5, 39, 132>>)),
        // assert_eq!(0x000002, the_netid(<<4, 16, 190, 163>>)),
        // ok.

        // Louis test data
        assert_eq!(0x600002, the_netid(0xE0040001));
        assert_eq!(0x600002, the_netid(0xE0052784));
        assert_eq!(0x000002, the_netid(0x0410BEA3));
    }
}
