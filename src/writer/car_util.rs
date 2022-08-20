use super::super::iroh_car;
use super::*;
use std::{borrow::Cow, collections::HashMap, mem};

use cid::Cid;
use ipld_pb::DagPbCodec;
use iroh_car::CarHeader;
use iroh_car::*;
use multihash::{Code::Blake2b256, MultihashDigest};
use quick_protobuf::message::MessageWrite;
use quick_protobuf::Writer;
use unixfs_v1::{PBLink, PBNode, UnixFs, UnixFsType};

const MAX_CAR_SIZE: usize = 104752742; // 99.9mb

trait ToVec {
    fn to_vec(&self) -> Vec<u8>;
}
impl<'a> ToVec for PBNode<'a> {
    fn to_vec(&self) -> Vec<u8> {
        let mut ret = vec![];
        let mut writer = Writer::new(&mut ret);
        self.write_message(&mut writer).unwrap();
        ret
    }
}
impl<'a> ToVec for UnixFs<'a> {
    fn to_vec(&self) -> Vec<u8> {
        let mut ret = vec![];
        let mut writer = Writer::new(&mut ret);
        self.write_message(&mut writer).unwrap();
        ret
    }
}

pub enum DirectoryItem {
    File(String, u64),
    Directory(String, Vec<DirectoryItem>),
}

impl DirectoryItem {
    fn to_unixfs_struct(&self, id_map: &HashMap<u64, &[UnixFsStruct]>) -> UnixFsStruct {
        match self {
            Self::File(name, id) => {
                if let Some(blocks) = id_map.get(id) {
                    gen_pbnode_from_blocks(name.clone(), blocks)
                } else {
                    empty_item()
                }
            },
            Self::Directory(name, sub_items) => {
                let items: Vec<UnixFsStruct> = sub_items
                    .iter()
                    .map(|x| x.to_unixfs_struct(id_map))
                    .collect();
                gen_dir(Some(name.clone()), &items)
            }
        }
    }
}

pub struct UnixFsStruct {
    name: Option<String>,
    cid: Cid,
    data: Vec<u8>,
    size: u64,
}
impl UnixFsStruct {
    pub fn to_link(&self) -> PBLink {
        PBLink {
            Name: None,
            Hash: Some(self.cid.to_bytes().into()),
            Tsize: Some(self.size),
        }
    }
}

fn empty_item() -> UnixFsStruct {
    let data_bytes = UnixFs {
        Type: UnixFsType::Raw,
        Data: None,
        filesize: None,
        blocksizes: vec![],
        hashType: None,
        fanout: None,
        mode: None,
        mtime: None,
    }
    .to_vec();

    let node_bytes = PBNode {
        Links: vec![],
        Data: Some(Cow::from(data_bytes)),
    }
    .to_vec();

    let digest = Blake2b256.digest(&node_bytes);
    let cid = Cid::new_v1(DagPbCodec.into(), digest);
    UnixFsStruct {
        name: None,
        cid,
        data: node_bytes,
        size: 0,
    }
}

fn gen_car(
    blocks: &mut [UnixFsStruct],
    unixfs_struct: Option<UnixFsStruct>,
) -> Result<Vec<u8>, iroh_car::Error> {
    let root = unixfs_struct.unwrap_or(empty_item());
    let header = CarHeader::new(vec![root.cid]);

    let mut buffer = Vec::with_capacity(MAX_CAR_SIZE);
    let mut writer = CarWriter::new(header, &mut buffer);

    for block in blocks {
        let data = mem::replace(&mut block.data, vec![]);
        writer.write(block.cid, data)?;
    }
    writer.write(root.cid, root.data)?;
    writer.flush()?;

    Ok(buffer)
}

fn gen_block(content: &[u8]) -> UnixFsStruct {
    let digest = Blake2b256.digest(content);
    let cid = Cid::new_v1(0x55, digest);
    UnixFsStruct {
        name: None,
        cid,
        data: content.to_vec(),
        size: content.len() as u64,
    }
}

fn gen_dir(name: Option<String>, items: &[UnixFsStruct]) -> UnixFsStruct {
    let data_bytes = UnixFs {
        Type: UnixFsType::Directory,
        Data: None,
        filesize: None,
        blocksizes: vec![],
        hashType: None,
        fanout: None,
        mode: None,
        mtime: None,
    }
    .to_vec();

    let mut dir_size = 0;
    let links = items
        .iter()
        .map(|x| {
            dir_size += x.size;
            PBLink {
                Name: x.name.as_ref().map(|x| Cow::from(x)),
                Hash: Some(Cow::from(x.cid.to_bytes())),
                Tsize: Some(x.size),
            }
        })
        .collect::<Vec<_>>();

    let node_bytes = PBNode {
        Links: links,
        Data: Some(Cow::from(data_bytes)),
    }
    .to_vec();

    let digest = Blake2b256.digest(&node_bytes);
    let cid = Cid::new_v1(DagPbCodec.into(), digest);

    UnixFsStruct {
        name,
        cid,
        data: node_bytes,
        size: dir_size,
    }
}

fn gen_pbnode_from_blocks(name: String, blocks: &[UnixFsStruct]) -> UnixFsStruct {
    let mut filesize = 0u64;
    let (links, blocksizes) = blocks
        .iter()
        .map(|x| {
            filesize += x.size;
            (x.to_link(), x.size)
        })
        .unzip();

    let data_bytes = UnixFs {
        Type: UnixFsType::File,
        Data: None,
        filesize: Some(filesize),
        blocksizes,
        hashType: None,
        fanout: None,
        mode: None,
        mtime: None,
    }
    .to_vec();

    let node_bytes = PBNode {
        Links: links,
        Data: Some(Cow::from(data_bytes)),
    }
    .to_vec();

    let digest = Blake2b256.digest(&node_bytes);
    let cid = Cid::new_v1(DagPbCodec.into(), digest);
    let size = node_bytes.len() as u64 + filesize;
    UnixFsStruct {
        name: Some(name),
        cid,
        data: node_bytes,
        size,
    }
}