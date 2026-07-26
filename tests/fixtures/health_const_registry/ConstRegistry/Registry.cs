namespace ConstRegistry;

/// A pure constants registry. Twenty-one names for literals, no behaviour and
/// no runtime state — the shape that used to be reported as a god class.
public static class Tuning
{
    public const float BaseSpeed = 1.0f;
    public const float SprintSpeed = 2.0f;
    public const float CrouchSpeed = 0.5f;
    public const float SwimSpeed = 0.75f;
    public const float ClimbSpeed = 0.6f;
    public const float JumpImpulse = 5.0f;
    public const float Gravity = -9.81f;
    public const float TerminalVelocity = -50.0f;
    public const float GroundFriction = 0.8f;
    public const float AirFriction = 0.1f;
    public const int MaxHealth = 100;
    public const int MaxStamina = 100;
    public const int MaxAmmo = 240;
    public const int MaxInventorySlots = 32;
    public const int MaxPartySize = 4;
    public const string DefaultProfile = "standard";
    public const string DebugProfile = "debug";
    public const string ReplayProfile = "replay";
    private const int TickRate = 60;
    private const int SnapshotRate = 20;
    internal const int SeedSalt = 7919;
}

/// Consts alongside real members. The consts drop out; the four members that
/// remain are what the threshold sees.
public class Mixed
{
    public const int Limit = 10;
    public const int Retries = 3;
    private const string Prefix = "mix";

    public int Count { get; set; }

    private int total;

    public void Add(int amount) { }

    public void Reset() { }
}

/// `static readonly` is not `const`. roe has no type analysis, so it cannot
/// tell a tuning value from a shared dependency, and these keep counting.
public static class Statics
{
    public static readonly string Name = "statics";
    public static readonly int[] Offsets = new int[] { 1, 2, 3 };
    private static readonly object Gate = new object();
    private static readonly string Suffix = "!";
}
