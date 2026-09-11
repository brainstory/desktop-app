// AudioWorklet processor that forwards raw mono Float32 frames to the main
// thread so they can be encoded into a 16 kHz PCM WAV file for whisper.
class PcmCollector extends AudioWorkletProcessor {
	process(inputs) {
		const input = inputs[0];
		if (input && input[0]) {
			// copy: the underlying buffer is reused by the audio graph
			this.port.postMessage(new Float32Array(input[0]));
		}
		return true;
	}
}

registerProcessor("pcm-collector", PcmCollector);
