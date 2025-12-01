# GStreamer DSC Plugin

A GStreamer plugin for Digital Signed Content (DSC) that provides cryptographic signing and verification for generic data based on OpenSSL signing and verification.

## Features

- **DSC Signer**: Signs data packets based on the specified hash method
- **DSC Verifier**: Verifies the signed data

## Generate certificate samples
```bash
openssl genrsa -out example_ca.key 4096
openssl rsa -in example_ca.key -pubout -out example_ca.pub
```

## Example
```bash
gst-launch-1.0 videotestsrc pattern=ball num-buffers=120 ! "video/x-raw,framerate=30/1" ! videoconvert ! x264enc ! dscsigner private-key-path= ./example_ca.key public-key-uri= ./example_ca.pub ! dscverifier key-store-path= `pwd` !  h264parse ! avdec_h264 ! videoconvert ! autovideosink
```
