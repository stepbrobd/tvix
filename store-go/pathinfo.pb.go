// SPDX-License-Identifier: MIT
// Copyright © 2022 The Tvix Authors

// Code generated by protoc-gen-go. DO NOT EDIT.
// versions:
// 	protoc-gen-go v1.36.4
// 	protoc        (unknown)
// source: tvix/store/protos/pathinfo.proto

package storev1

import (
	castore_go "code.tvl.fyi/tvix/castore-go"
	protoreflect "google.golang.org/protobuf/reflect/protoreflect"
	protoimpl "google.golang.org/protobuf/runtime/protoimpl"
	reflect "reflect"
	sync "sync"
	unsafe "unsafe"
)

const (
	// Verify that this generated code is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(20 - protoimpl.MinVersion)
	// Verify that runtime/protoimpl is sufficiently up-to-date.
	_ = protoimpl.EnforceVersion(protoimpl.MaxVersion - 20)
)

type NARInfo_CA_Hash int32

const (
	// produced when uploading fixed-output store paths using NAR-based
	// hashing (`outputHashMode = "recursive"`).
	NARInfo_CA_NAR_SHA256 NARInfo_CA_Hash = 0
	NARInfo_CA_NAR_SHA1   NARInfo_CA_Hash = 1
	NARInfo_CA_NAR_SHA512 NARInfo_CA_Hash = 2
	NARInfo_CA_NAR_MD5    NARInfo_CA_Hash = 3
	// Produced when uploading .drv files or outputs produced by
	// builtins.toFile.
	// Produces equivalent digests as FLAT_SHA256, but is a separate
	// hashing type in Nix, affecting output path calculation.
	NARInfo_CA_TEXT_SHA256 NARInfo_CA_Hash = 4
	// Produced when using fixed-output derivations with
	// `outputHashMode = "flat"`.
	NARInfo_CA_FLAT_SHA1   NARInfo_CA_Hash = 5
	NARInfo_CA_FLAT_MD5    NARInfo_CA_Hash = 6
	NARInfo_CA_FLAT_SHA256 NARInfo_CA_Hash = 7
	NARInfo_CA_FLAT_SHA512 NARInfo_CA_Hash = 8
)

// Enum value maps for NARInfo_CA_Hash.
var (
	NARInfo_CA_Hash_name = map[int32]string{
		0: "NAR_SHA256",
		1: "NAR_SHA1",
		2: "NAR_SHA512",
		3: "NAR_MD5",
		4: "TEXT_SHA256",
		5: "FLAT_SHA1",
		6: "FLAT_MD5",
		7: "FLAT_SHA256",
		8: "FLAT_SHA512",
	}
	NARInfo_CA_Hash_value = map[string]int32{
		"NAR_SHA256":  0,
		"NAR_SHA1":    1,
		"NAR_SHA512":  2,
		"NAR_MD5":     3,
		"TEXT_SHA256": 4,
		"FLAT_SHA1":   5,
		"FLAT_MD5":    6,
		"FLAT_SHA256": 7,
		"FLAT_SHA512": 8,
	}
)

func (x NARInfo_CA_Hash) Enum() *NARInfo_CA_Hash {
	p := new(NARInfo_CA_Hash)
	*p = x
	return p
}

func (x NARInfo_CA_Hash) String() string {
	return protoimpl.X.EnumStringOf(x.Descriptor(), protoreflect.EnumNumber(x))
}

func (NARInfo_CA_Hash) Descriptor() protoreflect.EnumDescriptor {
	return file_tvix_store_protos_pathinfo_proto_enumTypes[0].Descriptor()
}

func (NARInfo_CA_Hash) Type() protoreflect.EnumType {
	return &file_tvix_store_protos_pathinfo_proto_enumTypes[0]
}

func (x NARInfo_CA_Hash) Number() protoreflect.EnumNumber {
	return protoreflect.EnumNumber(x)
}

// Deprecated: Use NARInfo_CA_Hash.Descriptor instead.
func (NARInfo_CA_Hash) EnumDescriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{2, 1, 0}
}

// PathInfo shows information about a Nix Store Path.
// That's a single element inside /nix/store.
type PathInfo struct {
	state protoimpl.MessageState `protogen:"open.v1"`
	// The path can be a directory, file or symlink.
	Node *castore_go.Node `protobuf:"bytes,1,opt,name=node,proto3" json:"node,omitempty"`
	// List of references (output path hashes)
	// This really is the raw *bytes*, after decoding nixbase32, and not a
	// base32-encoded string.
	References [][]byte `protobuf:"bytes,2,rep,name=references,proto3" json:"references,omitempty"`
	// see below.
	Narinfo       *NARInfo `protobuf:"bytes,3,opt,name=narinfo,proto3" json:"narinfo,omitempty"`
	unknownFields protoimpl.UnknownFields
	sizeCache     protoimpl.SizeCache
}

func (x *PathInfo) Reset() {
	*x = PathInfo{}
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[0]
	ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
	ms.StoreMessageInfo(mi)
}

func (x *PathInfo) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*PathInfo) ProtoMessage() {}

func (x *PathInfo) ProtoReflect() protoreflect.Message {
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[0]
	if x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use PathInfo.ProtoReflect.Descriptor instead.
func (*PathInfo) Descriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{0}
}

func (x *PathInfo) GetNode() *castore_go.Node {
	if x != nil {
		return x.Node
	}
	return nil
}

func (x *PathInfo) GetReferences() [][]byte {
	if x != nil {
		return x.References
	}
	return nil
}

func (x *PathInfo) GetNarinfo() *NARInfo {
	if x != nil {
		return x.Narinfo
	}
	return nil
}

// Represents a path in the Nix store (a direct child of STORE_DIR).
// It is commonly formatted by a nixbase32-encoding the digest, and
// concatenating the name, separated by a `-`.
type StorePath struct {
	state protoimpl.MessageState `protogen:"open.v1"`
	// The string after digest and `-`.
	Name string `protobuf:"bytes,1,opt,name=name,proto3" json:"name,omitempty"`
	// The digest (20 bytes).
	Digest        []byte `protobuf:"bytes,2,opt,name=digest,proto3" json:"digest,omitempty"`
	unknownFields protoimpl.UnknownFields
	sizeCache     protoimpl.SizeCache
}

func (x *StorePath) Reset() {
	*x = StorePath{}
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[1]
	ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
	ms.StoreMessageInfo(mi)
}

func (x *StorePath) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*StorePath) ProtoMessage() {}

func (x *StorePath) ProtoReflect() protoreflect.Message {
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[1]
	if x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use StorePath.ProtoReflect.Descriptor instead.
func (*StorePath) Descriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{1}
}

func (x *StorePath) GetName() string {
	if x != nil {
		return x.Name
	}
	return ""
}

func (x *StorePath) GetDigest() []byte {
	if x != nil {
		return x.Digest
	}
	return nil
}

// Nix C++ uses NAR (Nix Archive) as a format to transfer store paths,
// and stores metadata and signatures in NARInfo files.
// Store all these attributes in a separate message.
//
// This is useful to render .narinfo files to clients, or to preserve/validate
// these signatures.
// As verifying these signatures requires the whole NAR file to be synthesized,
// moving to another signature scheme is desired.
// Even then, it still makes sense to hold this data, for old clients.
type NARInfo struct {
	state protoimpl.MessageState `protogen:"open.v1"`
	// This size of the NAR file, in bytes.
	NarSize uint64 `protobuf:"varint,1,opt,name=nar_size,json=narSize,proto3" json:"nar_size,omitempty"`
	// The sha256 of the NAR file representation.
	NarSha256 []byte `protobuf:"bytes,2,opt,name=nar_sha256,json=narSha256,proto3" json:"nar_sha256,omitempty"`
	// The signatures in a .narinfo file.
	Signatures []*NARInfo_Signature `protobuf:"bytes,3,rep,name=signatures,proto3" json:"signatures,omitempty"`
	// A list of references. To validate .narinfo signatures, a fingerprint needs
	// to be constructed.
	// This fingerprint doesn't just contain the hashes of the output paths of all
	// references (like PathInfo.references), but their whole (base)names, so we
	// need to keep them somewhere.
	ReferenceNames []string `protobuf:"bytes,4,rep,name=reference_names,json=referenceNames,proto3" json:"reference_names,omitempty"`
	// The StorePath of the .drv file producing this output.
	// The .drv suffix is omitted in its `name` field.
	Deriver *StorePath `protobuf:"bytes,5,opt,name=deriver,proto3" json:"deriver,omitempty"`
	// The CA field in the .narinfo.
	// Its textual representations seen in the wild are one of the following:
	//   - `fixed:r:sha256:1gcky5hlf5vqfzpyhihydmm54grhc94mcs8w7xr8613qsqb1v2j6`
	//     fixed-output derivations using "recursive" `outputHashMode`.
	//   - `fixed:sha256:19xqkh72crbcba7flwxyi3n293vav6d7qkzkh2v4zfyi4iia8vj8
	//     fixed-output derivations using "flat" `outputHashMode`
	//   - `text:sha256:19xqkh72crbcba7flwxyi3n293vav6d7qkzkh2v4zfyi4iia8vj8`
	//     Text hashing, used for uploaded .drv files and outputs produced by
	//     builtins.toFile.
	//
	// Semantically, they can be split into the following components:
	//   - "content address prefix". Currently, "fixed" and "text" are supported.
	//   - "hash mode". Currently, "flat" and "recursive" are supported.
	//   - "hash type". The underlying hash function used.
	//     Currently, sha1, md5, sha256, sha512.
	//   - "digest". The digest itself.
	//
	// There are some restrictions on the possible combinations.
	// For example, `text` and `fixed:recursive` always imply sha256.
	//
	// We use an enum to encode the possible combinations, and optimize for the
	// common case, `fixed:recursive`, identified as `NAR_SHA256`.
	Ca            *NARInfo_CA `protobuf:"bytes,6,opt,name=ca,proto3" json:"ca,omitempty"`
	unknownFields protoimpl.UnknownFields
	sizeCache     protoimpl.SizeCache
}

func (x *NARInfo) Reset() {
	*x = NARInfo{}
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[2]
	ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
	ms.StoreMessageInfo(mi)
}

func (x *NARInfo) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*NARInfo) ProtoMessage() {}

func (x *NARInfo) ProtoReflect() protoreflect.Message {
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[2]
	if x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use NARInfo.ProtoReflect.Descriptor instead.
func (*NARInfo) Descriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{2}
}

func (x *NARInfo) GetNarSize() uint64 {
	if x != nil {
		return x.NarSize
	}
	return 0
}

func (x *NARInfo) GetNarSha256() []byte {
	if x != nil {
		return x.NarSha256
	}
	return nil
}

func (x *NARInfo) GetSignatures() []*NARInfo_Signature {
	if x != nil {
		return x.Signatures
	}
	return nil
}

func (x *NARInfo) GetReferenceNames() []string {
	if x != nil {
		return x.ReferenceNames
	}
	return nil
}

func (x *NARInfo) GetDeriver() *StorePath {
	if x != nil {
		return x.Deriver
	}
	return nil
}

func (x *NARInfo) GetCa() *NARInfo_CA {
	if x != nil {
		return x.Ca
	}
	return nil
}

// This represents a (parsed) signature line in a .narinfo file.
type NARInfo_Signature struct {
	state         protoimpl.MessageState `protogen:"open.v1"`
	Name          string                 `protobuf:"bytes,1,opt,name=name,proto3" json:"name,omitempty"`
	Data          []byte                 `protobuf:"bytes,2,opt,name=data,proto3" json:"data,omitempty"`
	unknownFields protoimpl.UnknownFields
	sizeCache     protoimpl.SizeCache
}

func (x *NARInfo_Signature) Reset() {
	*x = NARInfo_Signature{}
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[3]
	ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
	ms.StoreMessageInfo(mi)
}

func (x *NARInfo_Signature) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*NARInfo_Signature) ProtoMessage() {}

func (x *NARInfo_Signature) ProtoReflect() protoreflect.Message {
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[3]
	if x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use NARInfo_Signature.ProtoReflect.Descriptor instead.
func (*NARInfo_Signature) Descriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{2, 0}
}

func (x *NARInfo_Signature) GetName() string {
	if x != nil {
		return x.Name
	}
	return ""
}

func (x *NARInfo_Signature) GetData() []byte {
	if x != nil {
		return x.Data
	}
	return nil
}

type NARInfo_CA struct {
	state protoimpl.MessageState `protogen:"open.v1"`
	// The hashing type used.
	Type NARInfo_CA_Hash `protobuf:"varint,1,opt,name=type,proto3,enum=tvix.store.v1.NARInfo_CA_Hash" json:"type,omitempty"`
	// The digest, in raw bytes.
	Digest        []byte `protobuf:"bytes,2,opt,name=digest,proto3" json:"digest,omitempty"`
	unknownFields protoimpl.UnknownFields
	sizeCache     protoimpl.SizeCache
}

func (x *NARInfo_CA) Reset() {
	*x = NARInfo_CA{}
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[4]
	ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
	ms.StoreMessageInfo(mi)
}

func (x *NARInfo_CA) String() string {
	return protoimpl.X.MessageStringOf(x)
}

func (*NARInfo_CA) ProtoMessage() {}

func (x *NARInfo_CA) ProtoReflect() protoreflect.Message {
	mi := &file_tvix_store_protos_pathinfo_proto_msgTypes[4]
	if x != nil {
		ms := protoimpl.X.MessageStateOf(protoimpl.Pointer(x))
		if ms.LoadMessageInfo() == nil {
			ms.StoreMessageInfo(mi)
		}
		return ms
	}
	return mi.MessageOf(x)
}

// Deprecated: Use NARInfo_CA.ProtoReflect.Descriptor instead.
func (*NARInfo_CA) Descriptor() ([]byte, []int) {
	return file_tvix_store_protos_pathinfo_proto_rawDescGZIP(), []int{2, 1}
}

func (x *NARInfo_CA) GetType() NARInfo_CA_Hash {
	if x != nil {
		return x.Type
	}
	return NARInfo_CA_NAR_SHA256
}

func (x *NARInfo_CA) GetDigest() []byte {
	if x != nil {
		return x.Digest
	}
	return nil
}

var File_tvix_store_protos_pathinfo_proto protoreflect.FileDescriptor

var file_tvix_store_protos_pathinfo_proto_rawDesc = string([]byte{
	0x0a, 0x20, 0x74, 0x76, 0x69, 0x78, 0x2f, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2f, 0x70, 0x72, 0x6f,
	0x74, 0x6f, 0x73, 0x2f, 0x70, 0x61, 0x74, 0x68, 0x69, 0x6e, 0x66, 0x6f, 0x2e, 0x70, 0x72, 0x6f,
	0x74, 0x6f, 0x12, 0x0d, 0x74, 0x76, 0x69, 0x78, 0x2e, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76,
	0x31, 0x1a, 0x21, 0x74, 0x76, 0x69, 0x78, 0x2f, 0x63, 0x61, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2f,
	0x70, 0x72, 0x6f, 0x74, 0x6f, 0x73, 0x2f, 0x63, 0x61, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x70,
	0x72, 0x6f, 0x74, 0x6f, 0x22, 0x87, 0x01, 0x0a, 0x08, 0x50, 0x61, 0x74, 0x68, 0x49, 0x6e, 0x66,
	0x6f, 0x12, 0x29, 0x0a, 0x04, 0x6e, 0x6f, 0x64, 0x65, 0x18, 0x01, 0x20, 0x01, 0x28, 0x0b, 0x32,
	0x15, 0x2e, 0x74, 0x76, 0x69, 0x78, 0x2e, 0x63, 0x61, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76,
	0x31, 0x2e, 0x4e, 0x6f, 0x64, 0x65, 0x52, 0x04, 0x6e, 0x6f, 0x64, 0x65, 0x12, 0x1e, 0x0a, 0x0a,
	0x72, 0x65, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x73, 0x18, 0x02, 0x20, 0x03, 0x28, 0x0c,
	0x52, 0x0a, 0x72, 0x65, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x73, 0x12, 0x30, 0x0a, 0x07,
	0x6e, 0x61, 0x72, 0x69, 0x6e, 0x66, 0x6f, 0x18, 0x03, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x16, 0x2e,
	0x74, 0x76, 0x69, 0x78, 0x2e, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76, 0x31, 0x2e, 0x4e, 0x41,
	0x52, 0x49, 0x6e, 0x66, 0x6f, 0x52, 0x07, 0x6e, 0x61, 0x72, 0x69, 0x6e, 0x66, 0x6f, 0x22, 0x37,
	0x0a, 0x09, 0x53, 0x74, 0x6f, 0x72, 0x65, 0x50, 0x61, 0x74, 0x68, 0x12, 0x12, 0x0a, 0x04, 0x6e,
	0x61, 0x6d, 0x65, 0x18, 0x01, 0x20, 0x01, 0x28, 0x09, 0x52, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x12,
	0x16, 0x0a, 0x06, 0x64, 0x69, 0x67, 0x65, 0x73, 0x74, 0x18, 0x02, 0x20, 0x01, 0x28, 0x0c, 0x52,
	0x06, 0x64, 0x69, 0x67, 0x65, 0x73, 0x74, 0x22, 0xa9, 0x04, 0x0a, 0x07, 0x4e, 0x41, 0x52, 0x49,
	0x6e, 0x66, 0x6f, 0x12, 0x19, 0x0a, 0x08, 0x6e, 0x61, 0x72, 0x5f, 0x73, 0x69, 0x7a, 0x65, 0x18,
	0x01, 0x20, 0x01, 0x28, 0x04, 0x52, 0x07, 0x6e, 0x61, 0x72, 0x53, 0x69, 0x7a, 0x65, 0x12, 0x1d,
	0x0a, 0x0a, 0x6e, 0x61, 0x72, 0x5f, 0x73, 0x68, 0x61, 0x32, 0x35, 0x36, 0x18, 0x02, 0x20, 0x01,
	0x28, 0x0c, 0x52, 0x09, 0x6e, 0x61, 0x72, 0x53, 0x68, 0x61, 0x32, 0x35, 0x36, 0x12, 0x40, 0x0a,
	0x0a, 0x73, 0x69, 0x67, 0x6e, 0x61, 0x74, 0x75, 0x72, 0x65, 0x73, 0x18, 0x03, 0x20, 0x03, 0x28,
	0x0b, 0x32, 0x20, 0x2e, 0x74, 0x76, 0x69, 0x78, 0x2e, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76,
	0x31, 0x2e, 0x4e, 0x41, 0x52, 0x49, 0x6e, 0x66, 0x6f, 0x2e, 0x53, 0x69, 0x67, 0x6e, 0x61, 0x74,
	0x75, 0x72, 0x65, 0x52, 0x0a, 0x73, 0x69, 0x67, 0x6e, 0x61, 0x74, 0x75, 0x72, 0x65, 0x73, 0x12,
	0x27, 0x0a, 0x0f, 0x72, 0x65, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x5f, 0x6e, 0x61, 0x6d,
	0x65, 0x73, 0x18, 0x04, 0x20, 0x03, 0x28, 0x09, 0x52, 0x0e, 0x72, 0x65, 0x66, 0x65, 0x72, 0x65,
	0x6e, 0x63, 0x65, 0x4e, 0x61, 0x6d, 0x65, 0x73, 0x12, 0x32, 0x0a, 0x07, 0x64, 0x65, 0x72, 0x69,
	0x76, 0x65, 0x72, 0x18, 0x05, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x18, 0x2e, 0x74, 0x76, 0x69, 0x78,
	0x2e, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76, 0x31, 0x2e, 0x53, 0x74, 0x6f, 0x72, 0x65, 0x50,
	0x61, 0x74, 0x68, 0x52, 0x07, 0x64, 0x65, 0x72, 0x69, 0x76, 0x65, 0x72, 0x12, 0x29, 0x0a, 0x02,
	0x63, 0x61, 0x18, 0x06, 0x20, 0x01, 0x28, 0x0b, 0x32, 0x19, 0x2e, 0x74, 0x76, 0x69, 0x78, 0x2e,
	0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76, 0x31, 0x2e, 0x4e, 0x41, 0x52, 0x49, 0x6e, 0x66, 0x6f,
	0x2e, 0x43, 0x41, 0x52, 0x02, 0x63, 0x61, 0x1a, 0x33, 0x0a, 0x09, 0x53, 0x69, 0x67, 0x6e, 0x61,
	0x74, 0x75, 0x72, 0x65, 0x12, 0x12, 0x0a, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x18, 0x01, 0x20, 0x01,
	0x28, 0x09, 0x52, 0x04, 0x6e, 0x61, 0x6d, 0x65, 0x12, 0x12, 0x0a, 0x04, 0x64, 0x61, 0x74, 0x61,
	0x18, 0x02, 0x20, 0x01, 0x28, 0x0c, 0x52, 0x04, 0x64, 0x61, 0x74, 0x61, 0x1a, 0xe4, 0x01, 0x0a,
	0x02, 0x43, 0x41, 0x12, 0x32, 0x0a, 0x04, 0x74, 0x79, 0x70, 0x65, 0x18, 0x01, 0x20, 0x01, 0x28,
	0x0e, 0x32, 0x1e, 0x2e, 0x74, 0x76, 0x69, 0x78, 0x2e, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2e, 0x76,
	0x31, 0x2e, 0x4e, 0x41, 0x52, 0x49, 0x6e, 0x66, 0x6f, 0x2e, 0x43, 0x41, 0x2e, 0x48, 0x61, 0x73,
	0x68, 0x52, 0x04, 0x74, 0x79, 0x70, 0x65, 0x12, 0x16, 0x0a, 0x06, 0x64, 0x69, 0x67, 0x65, 0x73,
	0x74, 0x18, 0x02, 0x20, 0x01, 0x28, 0x0c, 0x52, 0x06, 0x64, 0x69, 0x67, 0x65, 0x73, 0x74, 0x22,
	0x91, 0x01, 0x0a, 0x04, 0x48, 0x61, 0x73, 0x68, 0x12, 0x0e, 0x0a, 0x0a, 0x4e, 0x41, 0x52, 0x5f,
	0x53, 0x48, 0x41, 0x32, 0x35, 0x36, 0x10, 0x00, 0x12, 0x0c, 0x0a, 0x08, 0x4e, 0x41, 0x52, 0x5f,
	0x53, 0x48, 0x41, 0x31, 0x10, 0x01, 0x12, 0x0e, 0x0a, 0x0a, 0x4e, 0x41, 0x52, 0x5f, 0x53, 0x48,
	0x41, 0x35, 0x31, 0x32, 0x10, 0x02, 0x12, 0x0b, 0x0a, 0x07, 0x4e, 0x41, 0x52, 0x5f, 0x4d, 0x44,
	0x35, 0x10, 0x03, 0x12, 0x0f, 0x0a, 0x0b, 0x54, 0x45, 0x58, 0x54, 0x5f, 0x53, 0x48, 0x41, 0x32,
	0x35, 0x36, 0x10, 0x04, 0x12, 0x0d, 0x0a, 0x09, 0x46, 0x4c, 0x41, 0x54, 0x5f, 0x53, 0x48, 0x41,
	0x31, 0x10, 0x05, 0x12, 0x0c, 0x0a, 0x08, 0x46, 0x4c, 0x41, 0x54, 0x5f, 0x4d, 0x44, 0x35, 0x10,
	0x06, 0x12, 0x0f, 0x0a, 0x0b, 0x46, 0x4c, 0x41, 0x54, 0x5f, 0x53, 0x48, 0x41, 0x32, 0x35, 0x36,
	0x10, 0x07, 0x12, 0x0f, 0x0a, 0x0b, 0x46, 0x4c, 0x41, 0x54, 0x5f, 0x53, 0x48, 0x41, 0x35, 0x31,
	0x32, 0x10, 0x08, 0x42, 0x24, 0x5a, 0x22, 0x63, 0x6f, 0x64, 0x65, 0x2e, 0x74, 0x76, 0x6c, 0x2e,
	0x66, 0x79, 0x69, 0x2f, 0x74, 0x76, 0x69, 0x78, 0x2f, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x2d, 0x67,
	0x6f, 0x3b, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x76, 0x31, 0x62, 0x06, 0x70, 0x72, 0x6f, 0x74, 0x6f,
	0x33,
})

var (
	file_tvix_store_protos_pathinfo_proto_rawDescOnce sync.Once
	file_tvix_store_protos_pathinfo_proto_rawDescData []byte
)

func file_tvix_store_protos_pathinfo_proto_rawDescGZIP() []byte {
	file_tvix_store_protos_pathinfo_proto_rawDescOnce.Do(func() {
		file_tvix_store_protos_pathinfo_proto_rawDescData = protoimpl.X.CompressGZIP(unsafe.Slice(unsafe.StringData(file_tvix_store_protos_pathinfo_proto_rawDesc), len(file_tvix_store_protos_pathinfo_proto_rawDesc)))
	})
	return file_tvix_store_protos_pathinfo_proto_rawDescData
}

var file_tvix_store_protos_pathinfo_proto_enumTypes = make([]protoimpl.EnumInfo, 1)
var file_tvix_store_protos_pathinfo_proto_msgTypes = make([]protoimpl.MessageInfo, 5)
var file_tvix_store_protos_pathinfo_proto_goTypes = []any{
	(NARInfo_CA_Hash)(0),      // 0: tvix.store.v1.NARInfo.CA.Hash
	(*PathInfo)(nil),          // 1: tvix.store.v1.PathInfo
	(*StorePath)(nil),         // 2: tvix.store.v1.StorePath
	(*NARInfo)(nil),           // 3: tvix.store.v1.NARInfo
	(*NARInfo_Signature)(nil), // 4: tvix.store.v1.NARInfo.Signature
	(*NARInfo_CA)(nil),        // 5: tvix.store.v1.NARInfo.CA
	(*castore_go.Node)(nil),   // 6: tvix.castore.v1.Node
}
var file_tvix_store_protos_pathinfo_proto_depIdxs = []int32{
	6, // 0: tvix.store.v1.PathInfo.node:type_name -> tvix.castore.v1.Node
	3, // 1: tvix.store.v1.PathInfo.narinfo:type_name -> tvix.store.v1.NARInfo
	4, // 2: tvix.store.v1.NARInfo.signatures:type_name -> tvix.store.v1.NARInfo.Signature
	2, // 3: tvix.store.v1.NARInfo.deriver:type_name -> tvix.store.v1.StorePath
	5, // 4: tvix.store.v1.NARInfo.ca:type_name -> tvix.store.v1.NARInfo.CA
	0, // 5: tvix.store.v1.NARInfo.CA.type:type_name -> tvix.store.v1.NARInfo.CA.Hash
	6, // [6:6] is the sub-list for method output_type
	6, // [6:6] is the sub-list for method input_type
	6, // [6:6] is the sub-list for extension type_name
	6, // [6:6] is the sub-list for extension extendee
	0, // [0:6] is the sub-list for field type_name
}

func init() { file_tvix_store_protos_pathinfo_proto_init() }
func file_tvix_store_protos_pathinfo_proto_init() {
	if File_tvix_store_protos_pathinfo_proto != nil {
		return
	}
	type x struct{}
	out := protoimpl.TypeBuilder{
		File: protoimpl.DescBuilder{
			GoPackagePath: reflect.TypeOf(x{}).PkgPath(),
			RawDescriptor: unsafe.Slice(unsafe.StringData(file_tvix_store_protos_pathinfo_proto_rawDesc), len(file_tvix_store_protos_pathinfo_proto_rawDesc)),
			NumEnums:      1,
			NumMessages:   5,
			NumExtensions: 0,
			NumServices:   0,
		},
		GoTypes:           file_tvix_store_protos_pathinfo_proto_goTypes,
		DependencyIndexes: file_tvix_store_protos_pathinfo_proto_depIdxs,
		EnumInfos:         file_tvix_store_protos_pathinfo_proto_enumTypes,
		MessageInfos:      file_tvix_store_protos_pathinfo_proto_msgTypes,
	}.Build()
	File_tvix_store_protos_pathinfo_proto = out.File
	file_tvix_store_protos_pathinfo_proto_goTypes = nil
	file_tvix_store_protos_pathinfo_proto_depIdxs = nil
}
