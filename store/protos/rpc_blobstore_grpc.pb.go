// SPDX-License-Identifier: MIT
// Copyright © 2022 The Tvix Authors

// Code generated by protoc-gen-go-grpc. DO NOT EDIT.
// versions:
// - protoc-gen-go-grpc v1.3.0
// - protoc             (unknown)
// source: tvix/store/protos/rpc_blobstore.proto

package storev1

import (
	context "context"
	grpc "google.golang.org/grpc"
	codes "google.golang.org/grpc/codes"
	status "google.golang.org/grpc/status"
)

// This is a compile-time assertion to ensure that this generated file
// is compatible with the grpc package it is being compiled against.
// Requires gRPC-Go v1.32.0 or later.
const _ = grpc.SupportPackageIsVersion7

const (
	BlobService_Stat_FullMethodName = "/tvix.store.v1.BlobService/Stat"
	BlobService_Read_FullMethodName = "/tvix.store.v1.BlobService/Read"
	BlobService_Put_FullMethodName  = "/tvix.store.v1.BlobService/Put"
)

// BlobServiceClient is the client API for BlobService service.
//
// For semantics around ctx use and closing/ending streaming RPCs, please refer to https://pkg.go.dev/google.golang.org/grpc/?tab=doc#ClientConn.NewStream.
type BlobServiceClient interface {
	// Stat exposes metadata about a given blob,
	// such as more granular chunking, baos.
	// It implicitly allows checking for existence too, as asking this for a
	// non-existing Blob will return a Status::not_found grpc error.
	// If there's no more granular chunking available, the response will simply
	// contain a single chunk.
	Stat(ctx context.Context, in *StatBlobRequest, opts ...grpc.CallOption) (*BlobMeta, error)
	// Read returns a stream of BlobChunk, which is just a stream of bytes with
	// the digest specified in ReadBlobRequest.
	//
	// The server may decide on whatever chunking it may seem fit as a size for
	// the individual BlobChunk sent in the response stream.
	//
	// It specifically is NOT necessarily using chunk sizes communicated in a
	// previous Stat request.
	//
	// It's up to the specific store to decide on whether it allows Read on a
	// Blob at all, or only on smaller chunks communicated in a Stat() call
	// first.
	//
	// Clients are enouraged to Stat() first, and then only read the individual
	// chunks they don't have yet.
	Read(ctx context.Context, in *ReadBlobRequest, opts ...grpc.CallOption) (BlobService_ReadClient, error)
	// Put uploads a Blob, by reading a stream of bytes.
	//
	// The way the data is chunked up in individual BlobChunk messages sent in
	// the stream has no effect on how the server ends up chunking blobs up.
	Put(ctx context.Context, opts ...grpc.CallOption) (BlobService_PutClient, error)
}

type blobServiceClient struct {
	cc grpc.ClientConnInterface
}

func NewBlobServiceClient(cc grpc.ClientConnInterface) BlobServiceClient {
	return &blobServiceClient{cc}
}

func (c *blobServiceClient) Stat(ctx context.Context, in *StatBlobRequest, opts ...grpc.CallOption) (*BlobMeta, error) {
	out := new(BlobMeta)
	err := c.cc.Invoke(ctx, BlobService_Stat_FullMethodName, in, out, opts...)
	if err != nil {
		return nil, err
	}
	return out, nil
}

func (c *blobServiceClient) Read(ctx context.Context, in *ReadBlobRequest, opts ...grpc.CallOption) (BlobService_ReadClient, error) {
	stream, err := c.cc.NewStream(ctx, &BlobService_ServiceDesc.Streams[0], BlobService_Read_FullMethodName, opts...)
	if err != nil {
		return nil, err
	}
	x := &blobServiceReadClient{stream}
	if err := x.ClientStream.SendMsg(in); err != nil {
		return nil, err
	}
	if err := x.ClientStream.CloseSend(); err != nil {
		return nil, err
	}
	return x, nil
}

type BlobService_ReadClient interface {
	Recv() (*BlobChunk, error)
	grpc.ClientStream
}

type blobServiceReadClient struct {
	grpc.ClientStream
}

func (x *blobServiceReadClient) Recv() (*BlobChunk, error) {
	m := new(BlobChunk)
	if err := x.ClientStream.RecvMsg(m); err != nil {
		return nil, err
	}
	return m, nil
}

func (c *blobServiceClient) Put(ctx context.Context, opts ...grpc.CallOption) (BlobService_PutClient, error) {
	stream, err := c.cc.NewStream(ctx, &BlobService_ServiceDesc.Streams[1], BlobService_Put_FullMethodName, opts...)
	if err != nil {
		return nil, err
	}
	x := &blobServicePutClient{stream}
	return x, nil
}

type BlobService_PutClient interface {
	Send(*BlobChunk) error
	CloseAndRecv() (*PutBlobResponse, error)
	grpc.ClientStream
}

type blobServicePutClient struct {
	grpc.ClientStream
}

func (x *blobServicePutClient) Send(m *BlobChunk) error {
	return x.ClientStream.SendMsg(m)
}

func (x *blobServicePutClient) CloseAndRecv() (*PutBlobResponse, error) {
	if err := x.ClientStream.CloseSend(); err != nil {
		return nil, err
	}
	m := new(PutBlobResponse)
	if err := x.ClientStream.RecvMsg(m); err != nil {
		return nil, err
	}
	return m, nil
}

// BlobServiceServer is the server API for BlobService service.
// All implementations must embed UnimplementedBlobServiceServer
// for forward compatibility
type BlobServiceServer interface {
	// Stat exposes metadata about a given blob,
	// such as more granular chunking, baos.
	// It implicitly allows checking for existence too, as asking this for a
	// non-existing Blob will return a Status::not_found grpc error.
	// If there's no more granular chunking available, the response will simply
	// contain a single chunk.
	Stat(context.Context, *StatBlobRequest) (*BlobMeta, error)
	// Read returns a stream of BlobChunk, which is just a stream of bytes with
	// the digest specified in ReadBlobRequest.
	//
	// The server may decide on whatever chunking it may seem fit as a size for
	// the individual BlobChunk sent in the response stream.
	//
	// It specifically is NOT necessarily using chunk sizes communicated in a
	// previous Stat request.
	//
	// It's up to the specific store to decide on whether it allows Read on a
	// Blob at all, or only on smaller chunks communicated in a Stat() call
	// first.
	//
	// Clients are enouraged to Stat() first, and then only read the individual
	// chunks they don't have yet.
	Read(*ReadBlobRequest, BlobService_ReadServer) error
	// Put uploads a Blob, by reading a stream of bytes.
	//
	// The way the data is chunked up in individual BlobChunk messages sent in
	// the stream has no effect on how the server ends up chunking blobs up.
	Put(BlobService_PutServer) error
	mustEmbedUnimplementedBlobServiceServer()
}

// UnimplementedBlobServiceServer must be embedded to have forward compatible implementations.
type UnimplementedBlobServiceServer struct {
}

func (UnimplementedBlobServiceServer) Stat(context.Context, *StatBlobRequest) (*BlobMeta, error) {
	return nil, status.Errorf(codes.Unimplemented, "method Stat not implemented")
}
func (UnimplementedBlobServiceServer) Read(*ReadBlobRequest, BlobService_ReadServer) error {
	return status.Errorf(codes.Unimplemented, "method Read not implemented")
}
func (UnimplementedBlobServiceServer) Put(BlobService_PutServer) error {
	return status.Errorf(codes.Unimplemented, "method Put not implemented")
}
func (UnimplementedBlobServiceServer) mustEmbedUnimplementedBlobServiceServer() {}

// UnsafeBlobServiceServer may be embedded to opt out of forward compatibility for this service.
// Use of this interface is not recommended, as added methods to BlobServiceServer will
// result in compilation errors.
type UnsafeBlobServiceServer interface {
	mustEmbedUnimplementedBlobServiceServer()
}

func RegisterBlobServiceServer(s grpc.ServiceRegistrar, srv BlobServiceServer) {
	s.RegisterService(&BlobService_ServiceDesc, srv)
}

func _BlobService_Stat_Handler(srv interface{}, ctx context.Context, dec func(interface{}) error, interceptor grpc.UnaryServerInterceptor) (interface{}, error) {
	in := new(StatBlobRequest)
	if err := dec(in); err != nil {
		return nil, err
	}
	if interceptor == nil {
		return srv.(BlobServiceServer).Stat(ctx, in)
	}
	info := &grpc.UnaryServerInfo{
		Server:     srv,
		FullMethod: BlobService_Stat_FullMethodName,
	}
	handler := func(ctx context.Context, req interface{}) (interface{}, error) {
		return srv.(BlobServiceServer).Stat(ctx, req.(*StatBlobRequest))
	}
	return interceptor(ctx, in, info, handler)
}

func _BlobService_Read_Handler(srv interface{}, stream grpc.ServerStream) error {
	m := new(ReadBlobRequest)
	if err := stream.RecvMsg(m); err != nil {
		return err
	}
	return srv.(BlobServiceServer).Read(m, &blobServiceReadServer{stream})
}

type BlobService_ReadServer interface {
	Send(*BlobChunk) error
	grpc.ServerStream
}

type blobServiceReadServer struct {
	grpc.ServerStream
}

func (x *blobServiceReadServer) Send(m *BlobChunk) error {
	return x.ServerStream.SendMsg(m)
}

func _BlobService_Put_Handler(srv interface{}, stream grpc.ServerStream) error {
	return srv.(BlobServiceServer).Put(&blobServicePutServer{stream})
}

type BlobService_PutServer interface {
	SendAndClose(*PutBlobResponse) error
	Recv() (*BlobChunk, error)
	grpc.ServerStream
}

type blobServicePutServer struct {
	grpc.ServerStream
}

func (x *blobServicePutServer) SendAndClose(m *PutBlobResponse) error {
	return x.ServerStream.SendMsg(m)
}

func (x *blobServicePutServer) Recv() (*BlobChunk, error) {
	m := new(BlobChunk)
	if err := x.ServerStream.RecvMsg(m); err != nil {
		return nil, err
	}
	return m, nil
}

// BlobService_ServiceDesc is the grpc.ServiceDesc for BlobService service.
// It's only intended for direct use with grpc.RegisterService,
// and not to be introspected or modified (even as a copy)
var BlobService_ServiceDesc = grpc.ServiceDesc{
	ServiceName: "tvix.store.v1.BlobService",
	HandlerType: (*BlobServiceServer)(nil),
	Methods: []grpc.MethodDesc{
		{
			MethodName: "Stat",
			Handler:    _BlobService_Stat_Handler,
		},
	},
	Streams: []grpc.StreamDesc{
		{
			StreamName:    "Read",
			Handler:       _BlobService_Read_Handler,
			ServerStreams: true,
		},
		{
			StreamName:    "Put",
			Handler:       _BlobService_Put_Handler,
			ClientStreams: true,
		},
	},
	Metadata: "tvix/store/protos/rpc_blobstore.proto",
}
