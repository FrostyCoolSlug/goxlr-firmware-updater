<p align="center">
<h1>This tool may brick your GoXLR.<br />Proceed at your own risk</h1>
</p>

__GoXLR Firmware Updater__

This tool is a wizard which should apply a GoXLR firmware file to your device, it's based on reverse engineering the firmware process update, and uses the GoXLR Utility under the hood to make the changes.

Due to the dangerous nature of this code, I'm not going to provide pre-compiled binaries, you will also need to aquire the firmware update file (available with `!firmware` on discord), but it should for the most part
be compile and run.

All efforts have been taken to avoid breaking devices, but due to the nature of reverse engineering it's possible certain bits of error handling are missing, which could lead to the firmware update failing in an unrecoverable way. Use entirely at your own risk.
