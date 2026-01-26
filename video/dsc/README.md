# GStreamer DSC Plugin

A GStreamer plugin for Digitaly Signed Content (DSC) that provides cryptographic signing and verification for some encoded video data based on OpenSSL library. DSC allows to verify the authenticity of the videos.
They are mechanisms for trustworthy authentication and verification of video content have recently been developed by JVET for inclusion into these video coding standards. This has been realized by three new supplemental enhancement information (SEI) messages which enable to attach cryptographic signatures to flexible chunks of data of a video stream at the network abstraction layer (NAL) unit level.

The following are the references used for this implementation:
* Paper: https://www.hhi.fraunhofer.de/fileadmin/Events/2025/IBC_2025/IBC2025PaperAuthentication_HHI.pdf
* JVET Specs: https://www.jvet-experts.org/doc_end_user/documents/40_Geneva/wg11/JVET-AN1019-v1.zip
* DSC implementation in VVC VTM: https://vcgit.hhi.fraunhofer.de/jvet/VVCSoftware_VTM/-/releases/VTM-23.13

## Elements

- **DSC Signer**: Signs data packets based on the specified hash method
- **DSC Verifier**: Verifies the signed data

## Generate certificate samples
### Create CA
```bash
openssl genrsa -out example_ca.key 4096
openssl genrsa -aes256 -out example_ca.key 4096
openssl req -x509 -new -nodes -key example_ca.key -sha256 -days 1826 -out example_ca.crt
openssl x509 -in example_ca.crt -noout -pubkey -out example_ca.pub
```

### Create Content Provider certificate
```bash
openssl genrsa -out example_content.key 4096
openssl req -new -key example_content.key -out example_content.csr
openssl x509 -req -in example_content.csr -CA example_ca.crt -CAkey example_ca.key -out example_content.crt -days 730 -sha256
openssl x509 -in example_content.crt -noout -pubkey -out example_content.pub
```

## Example
```bash
gst-launch-1.0 videotestsrc pattern=ball num-buffers=30 ! "video/x-raw,framerate=30/1" ! videoconvert ! x264enc key-int-max=5 ! dscsigner private-key-path= ./example_content.key public-key-uri= ./example_content.crt substream-length=5 ! dscverifier key-store-path= `pwd` !  h264parse ! avdec_h264 ! videoconvert ! autovideosink
```
