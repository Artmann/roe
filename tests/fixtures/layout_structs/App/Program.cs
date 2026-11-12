using System.Numerics;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace App;

// Uploaded to a GPU buffer and measured with sizeof. Removing a field changes
// the size the GPU expects.
internal struct GpuDecal
{
    public Matrix4x4 Box;
    public uint Material;
    private uint _pad0;
    private uint _pad1;
    private uint _pad2;
}

// Passed to a native library that reads every field at a fixed offset.
[StructLayout(LayoutKind.Sequential)]
internal struct NativeDescription
{
    public nint Color;
    public nint Reactive;
    public float Sharpness;
}

// Explicit offsets on the fields, without [StructLayout] on the type.
internal struct Overlay
{
    [FieldOffset(0)]
    public int Whole;

    public short Low;
}

// Only ever handled through a pointer.
internal struct Header
{
    public int Magic;
    public int Version;
}

// Measured with Unsafe.SizeOf<T>().
internal struct Vertex
{
    public Vector3 Position;
    public Vector3 Normal;
}

// No layout evidence, but the field is named as padding.
internal struct Uniforms
{
    public Vector4 Tint;
    private float _padding;
    public float Reserved1;
}

// No layout evidence at all: an unused field here is still dead.
internal struct Plain
{
    public int Used;
    private int _unused;
}

internal static unsafe class Program
{
    [DllImport("native")]
    private static extern void Dispatch(NativeDescription* description);

    [DllImport("native")]
    private static extern void Read(Header* header);

    private static void Main()
    {
        var decals = new GpuDecal[4];
        decals[0] = new GpuDecal { Box = Matrix4x4.Identity, Material = 1 };
        Console.WriteLine(decals.Length * sizeof(GpuDecal));

        var description = new NativeDescription { Color = 1 };
        Dispatch(&description);

        var overlay = new Overlay();
        Console.WriteLine(overlay.Whole);

        var header = new Header();
        Read(&header);

        Console.WriteLine(Unsafe.SizeOf<Vertex>());

        var uniforms = new Uniforms { Tint = Vector4.One };
        Console.WriteLine(uniforms.Tint);

        var plain = new Plain { Used = 1 };
        Console.WriteLine(plain.Used);
    }
}
