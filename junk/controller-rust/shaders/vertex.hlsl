// 顶点着色器 - 简单的全屏四边形
Texture2D tex0 : register(t0);
SamplerState sampler0 : register(s0);

struct VSInput {
    float3 position : POSITION;
    float2 texcoord : TEXCOORD;
};

struct PSInput {
    float4 position : SV_POSITION;
    float2 texcoord : TEXCOORD;
};

PSInput VSMain(VSInput input) {
    PSInput output;
    output.position = float4(input.position, 1.0);
    output.texcoord = input.texcoord;
    return output;
}
