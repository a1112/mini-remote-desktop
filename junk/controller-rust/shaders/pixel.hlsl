// 像素着色器 - 纹理采样
Texture2D tex0 : register(t0);
SamplerState sampler0 : register(s0);

struct PSInput {
    float4 position : SV_POSITION;
    float2 texcoord : TEXCOORD;
};

float4 PSMain(PSInput input) : SV_TARGET {
    return tex0.Sample(sampler0, input.texcoord);
}
