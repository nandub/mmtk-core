using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text.Json;

internal static class Program
{
    private const uint MemCommit = 0x1000;
    private const uint MemReserve = 0x2000;
    private const uint MemDecommit = 0x4000;
    private const uint MemRelease = 0x8000;
    private const uint MemFree = 0x10000;
    private const uint PageNoAccess = 0x01;
    private const uint PageReadWrite = 0x04;
    private const long MiB = 1024L * 1024L;
    private const long KiB = 1024L;

    [StructLayout(LayoutKind.Sequential)]
    private struct SystemInfo
    {
        public uint ProcessorInfo;
        public uint PageSize;
        public nint MinimumApplicationAddress;
        public nint MaximumApplicationAddress;
        public nint ActiveProcessorMask;
        public uint NumberOfProcessors;
        public uint ProcessorType;
        public uint AllocationGranularity;
        public ushort ProcessorLevel;
        public ushort ProcessorRevision;
    }

    // x64 MEMORY_BASIC_INFORMATION; PartitionId is present on modern Windows.
    [StructLayout(LayoutKind.Sequential)]
    private struct MemoryBasicInformation
    {
        public nint BaseAddress;
        public nint AllocationBase;
        public uint AllocationProtect;
        public ushort PartitionId;
        public nuint RegionSize;
        public uint State;
        public uint Protect;
        public uint Type;
    }

    [DllImport("kernel32.dll")]
    private static extern void GetSystemInfo(out SystemInfo info);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern nint VirtualAlloc(nint address, nuint size, uint allocationType, uint protect);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool VirtualFree(nint address, nuint size, uint freeType);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern nuint VirtualQuery(nint address, out MemoryBasicInformation info, nuint length);

    private static void FailLastError(string action) => throw new Win32Exception(Marshal.GetLastWin32Error(), action);

    private static object Inspect(SystemInfo sys) => new
    {
        SchemaVersion = 1,
        GeneratedUtc = DateTimeOffset.UtcNow,
        Mode = "inspect",
        Platform = RuntimeInformation.OSDescription,
        Architecture = RuntimeInformation.ProcessArchitecture.ToString(),
        PointerBits = IntPtr.Size * 8,
        PageSizeBytes = sys.PageSize,
        AllocationGranularityBytes = sys.AllocationGranularity,
        MinimumApplicationAddress = Hex(sys.MinimumApplicationAddress),
        MaximumApplicationAddress = Hex(sys.MaximumApplicationAddress),
        QualificationStatus = "INFORMATIONAL ONLY - not CoreCLR compatibility certification"
    };

    private static string Hex(nint value) => $"0x{unchecked((ulong)value):X}";

    private static object Probe(SystemInfo sys, long requestedMiB, long requestedCommitKiB)
    {
        if (requestedMiB < 1 || requestedMiB > 4096)
            throw new ArgumentOutOfRangeException(nameof(requestedMiB), "Reservation must be 1..4096 MiB.");
        if (requestedCommitKiB < 4 || requestedCommitKiB > 1024)
            throw new ArgumentOutOfRangeException(nameof(requestedCommitKiB), "Commit must be 4..1024 KiB.");

        long size = checked(requestedMiB * MiB);
        long commit = checked(((requestedCommitKiB * KiB + sys.PageSize - 1) / sys.PageSize) * sys.PageSize);
        if (commit * 2 > size)
            throw new ArgumentException("Reservation must hold two separate commit windows.");

        nint reservation = 0;
        bool released = false;
        try
        {
            reservation = VirtualAlloc(0, checked((nuint)size), MemReserve, PageNoAccess);
            if (reservation == 0) FailLastError("VirtualAlloc(MEM_RESERVE)");

            nuint querySize = VirtualQuery(reservation, out MemoryBasicInformation initial, (nuint)Marshal.SizeOf<MemoryBasicInformation>());
            if (querySize == 0) FailLastError("VirtualQuery(reservation)");
            if (initial.AllocationBase != reservation || initial.State != MemReserve)
                throw new InvalidOperationException("Initial reservation ownership/state mismatch.");

            // Two committed windows: one at the beginning and one at the end.
            nint tail = reservation + checked((nint)(size - commit));
            if (VirtualAlloc(reservation, checked((nuint)commit), MemCommit, PageReadWrite) != reservation)
                FailLastError("VirtualAlloc(MEM_COMMIT,head)");
            if (VirtualAlloc(tail, checked((nuint)commit), MemCommit, PageReadWrite) != tail)
                FailLastError("VirtualAlloc(MEM_COMMIT,tail)");

            Marshal.WriteByte(reservation, 0, 0xA5);
            Marshal.WriteByte(tail, 0, 0x5A);
            if (!VirtualFree(reservation, checked((nuint)commit), MemDecommit))
                FailLastError("VirtualFree(MEM_DECOMMIT,head)");
            if (VirtualAlloc(reservation, checked((nuint)commit), MemCommit, PageReadWrite) != reservation)
                FailLastError("VirtualAlloc(MEM_COMMIT,recommit)");
            bool zeroed = Marshal.ReadByte(reservation, 0) == 0;
            bool neighborPreserved = Marshal.ReadByte(tail, 0) == 0x5A;
            if (!zeroed || !neighborPreserved)
                throw new InvalidOperationException("Recommit zeroing or neighbor preservation contract failed.");

            if (!VirtualFree(reservation, 0, MemRelease)) FailLastError("VirtualFree(MEM_RELEASE)");
            released = true;
            nuint afterQuerySize = VirtualQuery(reservation, out MemoryBasicInformation after, (nuint)Marshal.SizeOf<MemoryBasicInformation>());
            if (afterQuerySize == 0) FailLastError("VirtualQuery(after release)");
            bool freed = after.State == MemFree;
            if (!freed) throw new InvalidOperationException("Released range is not MEM_FREE.");

            return new
            {
                SchemaVersion = 1,
                GeneratedUtc = DateTimeOffset.UtcNow,
                Mode = "probe",
                Platform = RuntimeInformation.OSDescription,
                Architecture = RuntimeInformation.ProcessArchitecture.ToString(),
                PageSizeBytes = sys.PageSize,
                AllocationGranularityBytes = sys.AllocationGranularity,
                RequestedReservationMiB = requestedMiB,
                CommittedWindowKiB = commit / KiB,
                ReservationBase = Hex(reservation),
                InitialAllocationBase = Hex(initial.AllocationBase),
                InitialRegionSizeBytes = (ulong)initial.RegionSize,
                InitialState = initial.State,
                RecommitZeroed = zeroed,
                NeighborPreserved = neighborPreserved,
                ReleasedAndObservedFree = freed,
                QualificationStatus = "BOUNDED WIN32 PROBE ONLY - not MMTk/CoreCLR compatibility certification"
            };
        }
        finally
        {
            if (reservation != 0 && !released && !VirtualFree(reservation, 0, MemRelease))
                Console.Error.WriteLine($"WARNING: cleanup failed, Win32 error {Marshal.GetLastWin32Error()}, base={Hex(reservation)}");
        }
    }

    private static int Main(string[] args)
    {
        try
        {
            if (!OperatingSystem.IsWindows() || IntPtr.Size != 8)
                throw new PlatformNotSupportedException("This diagnostic requires a 64-bit Windows process.");
            if (args.Length == 0 || args[0] is "--help" or "-h")
            {
                Console.WriteLine("Usage: inspect [--json] | probe [--size-mib 1024] [--commit-kib 64] [--json]");
                Console.WriteLine("inspect makes no reservations; probe is opt-in and capped at 4096 MiB reserved, 1024 KiB per commit window.");
                return args.Length == 0 ? 2 : 0;
            }
            string mode = args[0];
            if (mode is not ("inspect" or "probe")) throw new ArgumentException($"Unknown mode: {mode}");
            long sizeMiB = 1024, commitKiB = 64;
            for (int i = 1; i < args.Length; i++)
            {
                if (args[i] == "--json") continue;
                if (args[i] is "--size-mib" or "--commit-kib")
                {
                    if (i + 1 >= args.Length || !long.TryParse(args[++i], out long value))
                        throw new ArgumentException("Expected an integer after size option.");
                    if (args[i - 1] == "--size-mib") sizeMiB = value;
                    else commitKiB = value;
                    continue;
                }
                throw new ArgumentException($"Unknown argument: {args[i]}");
            }
            if (mode == "inspect" && (sizeMiB != 1024 || commitKiB != 64))
                throw new ArgumentException("Size flags are valid only for probe.");
            GetSystemInfo(out SystemInfo info);
            object result = mode == "inspect" ? Inspect(info) : Probe(info, sizeMiB, commitKiB);
            Console.WriteLine(JsonSerializer.Serialize(result, new JsonSerializerOptions { WriteIndented = true }));
            return 0;
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine($"{exception.GetType().Name}: {exception.Message}");
            return 1;
        }
    }
}
