#!/usr/bin/env python3
"""
Test hardware encoding using container-based approach.
"""
import av
import numpy as np
import time
import io

print("="*60)
print("Hardware Encoder Test (Container Method)")
print("="*60)

# Create test frame (random image)
frame = av.VideoFrame.from_ndarray(
    np.random.randint(0, 255, (1080, 1920, 3), dtype=np.uint8),
    format='rgb24'
)

# Test encoders
encoders_to_test = []

# Check available codecs
for codec_name in av.codecs_available:
    if '264' in codec_name.lower():
        try:
            encoders_to_test.append(codec_name)
        except:
            pass

print(f"\nFound {len(encoders_to_test)} H.264 codecs")
print("Testing performance...\n")

results = []

for codec_name in encoders_to_test[:10]:  # Test first 10
    print(f"Testing {codec_name:<20} ... ", end="", flush=True)

    try:
        output = io.BytesIO()
        container = av.open(output, mode='w', format='h264')

        # Add stream with specific codec
        stream = container.add_stream(codec_name, rate=30)
        stream.width = 1920
        stream.height = 1080
        stream.bit_rate = 5_000_000

        # Test encoding 10 frames
        start = time.perf_counter()

        for _ in range(10):
            for packet in stream.encode(frame):
                container.mux(packet)

        # Flush
        for packet in stream.encode():
            container.mux(packet)

        container.close()
        elapsed = time.perf_counter() - start

        size = len(output.getvalue())
        fps = 10 / elapsed if elapsed > 0 else 0

        # Determine encoder type
        if 'nvenc' in codec_name:
            marker = '🚀'
            enc_type = 'NVIDIA GPU'
        elif 'qsv' in codec_name:
            marker = '⚡'
            enc_type = 'Intel QSV'
        elif 'amf' in codec_name:
            marker = '🔥'
            enc_type = 'AMD GPU'
        elif 'mf' in codec_name:
            marker = '📺'
            enc_type = 'Media Foundation'
        elif 'x264' in codec_name:
            marker = '💻'
            enc_type = 'Software'
        else:
            marker = '•'
            enc_type = 'Unknown'

        print(f"{marker} {fps:5.1f} FPS, {size/10:5.0f} bytes/frame ({enc_type})")
        results.append((codec_name, fps, size, enc_type))

    except Exception as e:
        print(f"❌ {str(e)[:40]}")

# Print summary
print("\n" + "="*60)
print("PERFORMANCE RANKING")
print("="*60)

if results:
    results.sort(key=lambda x: x[1], reverse=True)

    print(f"\n{'Encoder':<20} {'FPS':<10} {'Type':<15}")
    print("-"*50)
    for name, fps, size, enc_type in results:
        marker = "🚀" if fps > 100 else "⚡" if fps > 60 else "💻"
        print(f"{marker} {name:<20} {fps:<10.1f} {enc_type:<15}")

    # Find hardware encoder
    hw_encoders = [r for r in results if r[3] != 'Software']
    if hw_encoders:
        print(f"\n✅ Hardware encoding available!")
        print(f"   Best: {hw_encoders[0][0]} @ {hw_encoders[0][1]:.1f} FPS")
    else:
        print(f"\n⚠️  Only software encoding available")
        print(f"   Recommended: libx264 (preset=ultrafast)")
