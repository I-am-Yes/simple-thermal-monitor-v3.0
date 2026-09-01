using System.Diagnostics;
using System.Security.Principal;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.RegularExpressions;
using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.PawnIo;

internal static class Program
{
    private const string PawnIoSetupUrl =
        "https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe";

    public static int Main(string[] args)
    {
        Console.InputEncoding = Encoding.UTF8;
        Console.OutputEncoding = Encoding.UTF8;

        // Never block the first JSON response on PawnIO setup or CPU MSR init.
        new Thread(EnsurePawnIo) { IsBackground = true, Name = "pawnio-setup" }.Start();

        var computer = new Computer
        {
            IsCpuEnabled = false,
            IsGpuEnabled = true,
            IsStorageEnabled = false,
            IsMotherboardEnabled = false,
            IsControllerEnabled = false,
        };
        try
        {
            computer.Open();
        }
        catch
        {
            // GPU / storage can still be empty; the UI has user-mode fallbacks.
        }

        new Thread(CpuLoop) { IsBackground = true, Name = "lhm-cpu" }.Start();

        try
        {
            if (args.Contains("--dump", StringComparer.OrdinalIgnoreCase)
                || args.Contains("--once", StringComparer.OrdinalIgnoreCase))
            {
                WaitForCpu(TimeSpan.FromSeconds(6));
            }

            if (args.Contains("--dump", StringComparer.OrdinalIgnoreCase))
            {
                Dump(computer);
            }

            if (args.Contains("--once", StringComparer.OrdinalIgnoreCase))
            {
                WriteReading(ReadAll(computer));
                return 0;
            }

            string? line;
            while ((line = Console.ReadLine()) != null)
            {
                if (line.Equals("quit", StringComparison.OrdinalIgnoreCase))
                {
                    break;
                }

                WriteReading(ReadAll(computer));
            }
        }
        finally
        {
            computer.Close();
        }

        return 0;
    }

    private static void WriteReading(Reading reading)
    {
        Console.WriteLine(JsonSerializer.Serialize(reading, JsonOptions.Value));
        Console.Out.Flush();
    }

    private static readonly Lazy<JsonSerializerOptions> JsonOptions = new(() =>
        new JsonSerializerOptions
        {
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        });

    private static readonly object CpuCacheLock = new();
    private static float? cachedCpuPackage;
    private static float? cachedPCore0;
    private static readonly ManualResetEventSlim CpuReady = new(false);

    private static Reading ReadAll(Computer gpuAndStorage)
    {
        Reading reading = Read(gpuAndStorage);
        lock (CpuCacheLock)
        {
            reading.CpuPackage ??= cachedCpuPackage;
            reading.PCore0 ??= cachedPCore0;
        }

        return reading;
    }

    private static void CpuLoop()
    {
        var cpu = new Computer
        {
            IsCpuEnabled = true,
            IsGpuEnabled = false,
            IsStorageEnabled = false,
            IsMotherboardEnabled = false,
            IsControllerEnabled = false,
        };

        try
        {
            cpu.Open();
        }
        catch
        {
            CpuReady.Set();
            return;
        }

        try
        {
            while (true)
            {
                Reading reading = Read(cpu);
                lock (CpuCacheLock)
                {
                    cachedCpuPackage = reading.CpuPackage;
                    cachedPCore0 = reading.PCore0;
                }

                CpuReady.Set();
                Thread.Sleep(400);
            }
        }
        catch
        {
            CpuReady.Set();
        }
        finally
        {
            try
            {
                cpu.Close();
            }
            catch
            {
                // Ignore shutdown races.
            }
        }
    }

    private static void WaitForCpu(TimeSpan timeout)
    {
        CpuReady.Wait(timeout);
    }

    private static Reading Read(Computer computer)
    {
        try
        {
            return ReadUnsafe(computer);
        }
        catch
        {
            return new Reading { PawnIo = PawnIo.IsInstalled };
        }
    }

    private static Reading ReadUnsafe(Computer computer)
    {
        float? cpuPackage = null;
        float? pCore0 = null;
        float? gpu = null;
        float? ssd = null;
        float? nvme = null;

        foreach (IHardware hardware in computer.Hardware)
        {
            UpdateTree(hardware);
            Collect(hardware, ref cpuPackage, ref pCore0, ref gpu, ref ssd, ref nvme);
        }

        return new Reading
        {
            CpuPackage = cpuPackage,
            PCore0 = pCore0,
            Gpu = gpu,
            Ssd = nvme ?? ssd,
            PawnIo = PawnIo.IsInstalled,
        };
    }

    private static void UpdateTree(IHardware hardware)
    {
        hardware.Update();
        foreach (IHardware sub in hardware.SubHardware)
        {
            UpdateTree(sub);
        }
    }

    private static void Collect(
        IHardware hardware,
        ref float? cpuPackage,
        ref float? pCore0,
        ref float? gpu,
        ref float? ssd,
        ref float? nvme)
    {
        foreach (ISensor sensor in hardware.Sensors)
        {
            if (sensor.SensorType != SensorType.Temperature || !sensor.Value.HasValue)
            {
                continue;
            }

            float temp = sensor.Value.Value;
            if (temp is < 1f or > 125f)
            {
                continue;
            }

            switch (hardware.HardwareType)
            {
                case HardwareType.Cpu:
                    if (IsCpuPackage(sensor.Name))
                    {
                        cpuPackage = temp;
                    }
                    else if (pCore0 is null && IsPCore0(sensor.Name))
                    {
                        pCore0 = temp;
                    }
                    break;
                case HardwareType.GpuNvidia:
                case HardwareType.GpuAmd:
                    if (gpu is null || sensor.Name.Contains("Core", StringComparison.OrdinalIgnoreCase))
                    {
                        gpu = temp;
                    }
                    break;
                case HardwareType.GpuIntel:
                    if (gpu is null)
                    {
                        gpu = temp;
                    }
                    break;
                case HardwareType.Storage:
                {
                    bool prefer = hardware.Name.Contains("NVMe", StringComparison.OrdinalIgnoreCase)
                        || hardware.Name.Contains("SSD", StringComparison.OrdinalIgnoreCase)
                        || hardware.Name.Contains("Solid", StringComparison.OrdinalIgnoreCase);
                    if (prefer)
                    {
                        nvme = nvme is null ? temp : Math.Max(nvme.Value, temp);
                    }
                    else
                    {
                        ssd = ssd is null ? temp : Math.Max(ssd.Value, temp);
                    }
                    break;
                }
            }
        }

        foreach (IHardware sub in hardware.SubHardware)
        {
            Collect(sub, ref cpuPackage, ref pCore0, ref gpu, ref ssd, ref nvme);
        }
    }

    private static bool IsCpuPackage(string name) =>
        name.Equals("CPU Package", StringComparison.OrdinalIgnoreCase)
        || name.Contains("Package", StringComparison.OrdinalIgnoreCase);

    private static bool IsPCore0(string name)
    {
        if (name.Contains("E-Core", StringComparison.OrdinalIgnoreCase)
            || name.Contains("ECore", StringComparison.OrdinalIgnoreCase)
            || name.Contains("Package", StringComparison.OrdinalIgnoreCase)
            || name.Contains("Average", StringComparison.OrdinalIgnoreCase)
            || name.Contains("Max", StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        if (Regex.IsMatch(name, @"P-?Core\s*#?\s*[01]\b", RegexOptions.IgnoreCase))
        {
            return true;
        }

        return Regex.IsMatch(name, @"CPU\s*Core\s*#\s*1\b", RegexOptions.IgnoreCase)
            || name.Equals("Core #1", StringComparison.OrdinalIgnoreCase);
    }

    private static void Dump(Computer computer)
    {
        foreach (IHardware hardware in computer.Hardware)
        {
            UpdateTree(hardware);
            DumpHardware(hardware, "");
        }
    }

    private static void DumpHardware(IHardware hardware, string indent)
    {
        Console.Error.WriteLine($"{indent}{hardware.HardwareType}: {hardware.Name}");
        foreach (ISensor sensor in hardware.Sensors)
        {
            if (sensor.SensorType != SensorType.Temperature)
            {
                continue;
            }

            Console.Error.WriteLine($"{indent}  {sensor.Name} = {sensor.Value}");
        }

        foreach (IHardware sub in hardware.SubHardware)
        {
            DumpHardware(sub, indent + "  ");
        }
    }

    private static void EnsurePawnIo()
    {
        if (PawnIo.IsInstalled || !IsElevated())
        {
            return;
        }

        try
        {
            string setup = Path.Combine(AppContext.BaseDirectory, "PawnIO_setup.exe");
            if (!File.Exists(setup))
            {
                setup = Path.Combine(Path.GetTempPath(), "PawnIO_setup.exe");
                using var http = new HttpClient { Timeout = TimeSpan.FromMinutes(2) };
                byte[] bytes = http.GetByteArrayAsync(PawnIoSetupUrl).GetAwaiter().GetResult();
                File.WriteAllBytes(setup, bytes);
            }

            using var process = Process.Start(
                new ProcessStartInfo
                {
                    FileName = setup,
                    Arguments = "-install -silent",
                    UseShellExecute = false,
                    CreateNoWindow = true,
                });
            process?.WaitForExit(120_000);
        }
        catch
        {
            // Keep running; user-mode fallbacks still work without PawnIO.
        }
    }

    private static bool IsElevated()
    {
        using var identity = WindowsIdentity.GetCurrent();
        return new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
    }

    private sealed class Reading
    {
        public float? CpuPackage { get; set; }
        public float? PCore0 { get; set; }
        public float? Gpu { get; set; }
        public float? Ssd { get; set; }
        public bool PawnIo { get; set; }
    }
}
