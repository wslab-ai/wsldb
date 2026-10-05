use super::{HEADER_SIZE, LeafEntry, LeafValue, Node, PAGE_SIZE};
use crate::domain::PageId;
use crate::{Error, TransactionId};

#[test]
fn leaf_round_trip() {
    let node = Node::Leaf(vec![
        LeafEntry {
            key: b"alpha".to_vec().into_boxed_slice(),
            value: LeafValue::Inline(b"one".to_vec().into_boxed_slice()),
        },
        LeafEntry {
            key: b"beta".to_vec().into_boxed_slice(),
            value: LeafValue::Inline(b"two".to_vec().into_boxed_slice()),
        },
    ]);
    let transaction_id = TransactionId::from_raw(4);
    let page_id = PageId::from_raw(9).unwrap();
    let page = node.encode(page_id, transaction_id).unwrap();
    assert_eq!(
        &page[..65],
        &[
            0x57, 0x53, 0x4c, 0x50, 0x00, 0x01, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x04, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xec, 0x8a, 0x97, 0x35, 0x9f, 0xc0, 0x0c, 0xba,
            0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x05, 0x00, 0x61, 0x6c, 0x70, 0x68,
            0x61, 0x01, 0x03, 0x00, 0x00, 0x00, 0x6f, 0x6e, 0x65, 0x04, 0x00, 0x62, 0x65, 0x74,
            0x61, 0x01, 0x03, 0x00, 0x00, 0x00, 0x74, 0x77, 0x6f,
        ]
    );
    let decoded = Node::decode(&page, page_id, transaction_id).unwrap();
    assert_eq!(decoded.encode(page_id, transaction_id).unwrap(), page);
    assert_eq!(
        decoded.leaf_entries_owned().unwrap(),
        node.leaf_entries_owned().unwrap()
    );
}

#[test]
fn leaf_common_prefix_is_stored_once() {
    let entries = vec![
        LeafEntry {
            key: b"shared-prefix-0001".to_vec().into_boxed_slice(),
            value: LeafValue::Inline(b"one".to_vec().into_boxed_slice()),
        },
        LeafEntry {
            key: b"shared-prefix-0002".to_vec().into_boxed_slice(),
            value: LeafValue::Inline(b"two".to_vec().into_boxed_slice()),
        },
    ];
    let uncompressed_len = 2 + entries
        .iter()
        .map(|entry| 2 + entry.key.len() + entry.value.encoded_len())
        .sum::<usize>();
    assert!(Node::leaf_encoded_len(&entries) < uncompressed_len);

    let node = Node::Leaf(entries);
    let transaction_id = TransactionId::from_raw(4);
    let page_id = PageId::from_raw(9).unwrap();
    let page = node.encode(page_id, transaction_id).unwrap();
    let decoded = Node::decode(&page, page_id, transaction_id).unwrap();
    assert_eq!(decoded.encode(page_id, transaction_id).unwrap(), page);
    assert_eq!(
        decoded.leaf_entries_owned().unwrap(),
        node.leaf_entries_owned().unwrap()
    );
}

#[test]
fn overflow_round_trip() {
    let node = Node::Overflow {
        next: PageId::from_raw(8),
        bytes: vec![7; 128],
    };
    let transaction_id = TransactionId::from_raw(4);
    let page_id = PageId::from_raw(9).unwrap();
    let page = node.encode(page_id, transaction_id).unwrap();
    assert_eq!(Node::decode(&page, page_id, transaction_id).unwrap(), node);
}

#[test]
fn nonzero_reserved_page_bytes_are_rejected() {
    let node = Node::Leaf(vec![LeafEntry {
        key: b"key".to_vec().into_boxed_slice(),
        value: LeafValue::Inline(b"value".to_vec().into_boxed_slice()),
    }]);
    let transaction_id = TransactionId::from_raw(1);
    for offset in [4, 6, 28, PAGE_SIZE - 1] {
        let page_id = PageId::from_raw(9).unwrap();
        let mut page = node.encode(page_id, transaction_id).unwrap();
        page[offset] = 1;
        assert!(matches!(
            Node::decode(&page, page_id, transaction_id),
            Err(Error::Corrupt(_))
        ));
    }
}

#[test]
fn page_cannot_be_relocated_without_detection() {
    let node = Node::Leaf(vec![LeafEntry {
        key: b"key".to_vec().into_boxed_slice(),
        value: LeafValue::Inline(b"value".to_vec().into_boxed_slice()),
    }]);
    let transaction_id = TransactionId::from_raw(1);
    let original = PageId::from_raw(9).unwrap();
    let relocated = PageId::from_raw(10).unwrap();
    let page = node.encode(original, transaction_id).unwrap();

    assert!(matches!(
        Node::decode(&page, relocated, transaction_id),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn impossible_entry_counts_are_rejected_before_collection_allocation() {
    let transaction_id = TransactionId::from_raw(1);
    let page_id = PageId::from_raw(9).unwrap();
    let leaf = Node::Leaf(vec![LeafEntry {
        key: b"key".to_vec().into_boxed_slice(),
        value: LeafValue::Inline(b"value".to_vec().into_boxed_slice()),
    }]);
    let mut leaf_page = leaf.encode(page_id, transaction_id).unwrap();
    leaf_page[HEADER_SIZE..HEADER_SIZE + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    rewrite_checksums(&mut leaf_page, page_id);
    assert!(matches!(
        Node::decode(&leaf_page, page_id, transaction_id),
        Err(Error::Corrupt(reason)) if reason == "leaf entry count exceeds its payload"
    ));

    let branch = Node::Branch {
        children: vec![PageId::from_raw(2).unwrap(), PageId::from_raw(3).unwrap()],
        separators: vec![b"middle".to_vec()],
    };
    let mut branch_page = branch.encode(page_id, transaction_id).unwrap();
    branch_page[HEADER_SIZE..HEADER_SIZE + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    rewrite_checksums(&mut branch_page, page_id);
    assert!(matches!(
        Node::decode(&branch_page, page_id, transaction_id),
        Err(Error::Corrupt(reason)) if reason == "branch child count exceeds its payload"
    ));
}

fn rewrite_checksums(page: &mut [u8; PAGE_SIZE], page_id: PageId) {
    let payload_len = u32::from_le_bytes(page[8..12].try_into().unwrap()) as usize;
    let payload_checksum = super::crc32c(&page[HEADER_SIZE..HEADER_SIZE + payload_len]);
    page[20..24].copy_from_slice(&payload_checksum.to_le_bytes());
    let header_checksum = super::header_checksum(page, page_id);
    page[24..28].copy_from_slice(&header_checksum.to_le_bytes());
}
