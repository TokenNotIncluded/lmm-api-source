$ErrorActionPreference = 'Stop'
# Control-only timing; only fixed phases and a monotonic integer leave stderr.
try { $journalPhaseClock = [System.Diagnostics.Stopwatch]::StartNew() } catch { $journalPhaseClock = $null }
function Write-JournalPhase([string] $phase) {
    try {
        if ($null -eq $journalPhaseClock) { return }
        if ($phase -notin @('script_enter', 'input_ready', 'compile_begin', 'compile_end', 'storage_begin', 'storage_end', 'result_ready')) { return }
        $elapsed = $journalPhaseClock.ElapsedMilliseconds.ToString([System.Globalization.CultureInfo]::InvariantCulture)
        [Console]::Error.WriteLine('LMM_JOURNAL_PHASE_V1 ' + $phase + ' ' + $elapsed)
        [Console]::Error.Flush()
    } catch { }
}
Write-JournalPhase 'script_enter'
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

# This fixed program is package code. Paths and the credential-free record arrive
# as JSON on stdin; they are never interpolated into PowerShell or C# source.
$source = @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;
using Microsoft.Win32.SafeHandles;

public sealed class LmmMagpieJournalResult {
    public bool Exists;
    public string Content;
}

public static class LmmMagpieJournalWindows {
    const uint ReadControl = 0x00020000;
    const uint ReadAttributes = 0x00000080;
    const uint GenericReadWrite = 0xC0000000;
    const uint ShareRead = 1;
    const uint CreateNew = 1;
    const uint OpenExisting = 3;
    const uint OpenReparsePoint = 0x00200000;
    const uint BackupSemantics = 0x02000000;
    const uint WriteThrough = 0x80000000;
    const uint DirectoryAttribute = 0x10;
    const uint ReparseAttribute = 0x400;

    [StructLayout(LayoutKind.Sequential)]
    struct SecurityAttributes {
        public int Length;
        public IntPtr Descriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool InheritHandle;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct FileInformation {
        public uint Attributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME AccessTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME WriteTime;
        public uint VolumeSerialNumber;
        public uint SizeHigh;
        public uint SizeLow;
        public uint Links;
        public uint IndexHigh;
        public uint IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CreateDirectoryW(string path, ref SecurityAttributes attributes);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
        IntPtr attributes, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true, EntryPoint="CreateFileW")]
    static extern SafeFileHandle CreatePrivateFile(string path, uint access, uint share,
        ref SecurityAttributes attributes, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true, ExactSpelling=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInformation info);
    [DllImport("advapi32.dll", SetLastError=true, ExactSpelling=true)]
    static extern uint GetSecurityInfo(SafeFileHandle handle, int objectType, uint information,
        out IntPtr owner, out IntPtr group, out IntPtr dacl, out IntPtr sacl, out IntPtr descriptor);
    [DllImport("advapi32.dll", ExactSpelling=true)]
    static extern uint GetSecurityDescriptorLength(IntPtr descriptor);
    [DllImport("kernel32.dll", ExactSpelling=true)]
    static extern IntPtr LocalFree(IntPtr memory);

    static void Need(bool condition) {
        if (!condition) throw new IOException("Unverified journal storage.");
    }
    static string NativePath(string path) { return @"\\?\" + path; }
    static SecurityAttributes PrivateSecurity(SecurityIdentifier sid, bool directory) {
        FileSystemAccessRule rule = new FileSystemAccessRule(sid, FileSystemRights.FullControl,
            directory ? InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit : InheritanceFlags.None,
            PropagationFlags.None, AccessControlType.Allow);
        FileSystemSecurity security;
        if (directory) {
            DirectorySecurity value = new DirectorySecurity();
            value.AddAccessRule(rule); security = value;
        } else {
            FileSecurity value = new FileSecurity();
            value.AddAccessRule(rule); security = value;
        }
        security.SetOwner(sid);
        security.SetAccessRuleProtection(true, false);
        byte[] bytes = security.GetSecurityDescriptorBinaryForm();
        IntPtr descriptor = Marshal.AllocHGlobal(bytes.Length);
        Marshal.Copy(bytes, 0, descriptor, bytes.Length);
        return new SecurityAttributes {
            Length = Marshal.SizeOf(typeof(SecurityAttributes)),
            Descriptor = descriptor,
            InheritHandle = false
        };
    }
    static FileInformation Information(SafeFileHandle handle, bool directory) {
        FileInformation info;
        if (!GetFileInformationByHandle(handle, out info)) throw new Win32Exception(Marshal.GetLastWin32Error());
        Need((info.Attributes & ReparseAttribute) == 0);
        Need(((info.Attributes & DirectoryAttribute) != 0) == directory);
        return info;
    }
    static void CheckSecurity(SafeFileHandle handle, SecurityIdentifier sid, bool directory) {
        IntPtr owner, group, dacl, sacl, descriptor;
        uint error = GetSecurityInfo(handle, 1, 5, out owner, out group, out dacl, out sacl, out descriptor);
        if (error != 0) throw new Win32Exception((int)error);
        try {
            uint length = GetSecurityDescriptorLength(descriptor);
            Need(length > 0 && length <= 65536);
            byte[] bytes = new byte[(int)length];
            Marshal.Copy(descriptor, bytes, 0, bytes.Length);
            RawSecurityDescriptor security = new RawSecurityDescriptor(bytes, 0);
            Need(security.Owner != null && security.Owner.Equals(sid));
            Need((security.ControlFlags & ControlFlags.DiscretionaryAclProtected) != 0);
            Need((security.ControlFlags & ControlFlags.DiscretionaryAclPresent) != 0);
            Need(security.DiscretionaryAcl != null && security.DiscretionaryAcl.Count == 1);
            CommonAce rule = security.DiscretionaryAcl[0] as CommonAce;
            AceFlags expected = directory ? AceFlags.ContainerInherit | AceFlags.ObjectInherit : AceFlags.None;
            Need(rule != null && !rule.IsCallback && rule.AceQualifier == AceQualifier.AccessAllowed);
            Need(rule.SecurityIdentifier.Equals(sid) && rule.AccessMask == (int)FileSystemRights.FullControl);
            Need(rule.AceFlags == expected);
        } finally { LocalFree(descriptor); }
    }
    static SafeFileHandle OpenDirectory(string path) {
        // Holding each ancestor without write/delete sharing prevents replacement
        // while the child record is opened. Inability to hold it is a refusal.
        SafeFileHandle handle = CreateFileW(NativePath(path), ReadControl | ReadAttributes,
            ShareRead, IntPtr.Zero, OpenExisting, OpenReparsePoint | BackupSemantics, IntPtr.Zero);
        if (handle.IsInvalid) {
            int error = Marshal.GetLastWin32Error(); handle.Dispose(); throw new Win32Exception(error);
        }
        try { Information(handle, true); return handle; }
        catch { handle.Dispose(); throw; }
    }
    static string[] Segments(string directory) {
        Need(directory != null && directory.Length >= 4 && directory.Length <= 4096);
        Need(((directory[0] >= 'A' && directory[0] <= 'Z') || (directory[0] >= 'a' && directory[0] <= 'z'))
            && directory[1] == ':' && directory[2] == '\\');
        string[] segments = directory.Substring(3).Split('\\');
        foreach (string segment in segments) {
            Need(segment.Length > 0 && segment != "." && segment != "..");
            Need(!segment.EndsWith(" ") && !segment.EndsWith("."));
            foreach (char value in segment) Need(value >= 32 && "<>:\"/|?*".IndexOf(value) < 0);
            string stem = segment.Split('.')[0].ToUpperInvariant();
            Need(stem != "CON" && stem != "PRN" && stem != "AUX" && stem != "NUL");
            Need(!(stem.Length == 4 && (stem.StartsWith("COM") || stem.StartsWith("LPT"))
                && stem[3] >= '1' && stem[3] <= '9'));
        }
        return segments;
    }
    public static LmmMagpieJournalResult Run(string directory, string name, string action, string content) {
        string[] segments = Segments(directory);
        Need(name != null && name.Length == 69 && name.EndsWith(".json"));
        for (int i = 0; i < 64; i++) Need((name[i] >= '0' && name[i] <= '9') || (name[i] >= 'a' && name[i] <= 'f'));
        Need(action == "pending" || action == "begin");
        UTF8Encoding utf8 = new UTF8Encoding(false, true);
        byte[] supplied = action == "pending" || content == null ? null : utf8.GetBytes(content);
        Need(action == "pending" ? String.IsNullOrEmpty(content) : supplied != null && supplied.Length > 0 && supplied.Length < 256);
        DriveInfo drive = new DriveInfo(directory.Substring(0, 3));
        Need(drive.DriveType == DriveType.Fixed && String.Equals(drive.DriveFormat, "NTFS", StringComparison.OrdinalIgnoreCase));
        SecurityIdentifier sid = WindowsIdentity.GetCurrent().User;
        Need(sid != null);
        SecurityAttributes directorySecurity = PrivateSecurity(sid, true);
        SecurityAttributes fileSecurity = PrivateSecurity(sid, false);
        List<SafeFileHandle> parents = new List<SafeFileHandle>();
        try {
            string path = directory.Substring(0, 3);
            parents.Add(OpenDirectory(path));
            for (int i = 0; i < segments.Length; i++) {
                path = Path.Combine(path, segments[i]);
                bool created = CreateDirectoryW(NativePath(path), ref directorySecurity);
                int error = created ? 0 : Marshal.GetLastWin32Error();
                if (!created && error != 183) throw new Win32Exception(error);
                SafeFileHandle parent = OpenDirectory(path);
                parents.Add(parent);
                // Existing ancestors may have their normal system ACL; every new
                // directory and the journal directory must be private. Never fix
                // or replace an existing ACL.
                if (created || i == segments.Length - 1) CheckSecurity(parent, sid, true);
            }
            string recordPath = NativePath(Path.Combine(directory, name));
            bool createdRecord = false;
            SafeFileHandle record;
            if (action == "begin") {
                record = CreatePrivateFile(recordPath, GenericReadWrite | ReadControl,
                    ShareRead, ref fileSecurity, CreateNew, OpenReparsePoint | WriteThrough, IntPtr.Zero);
                createdRecord = !record.IsInvalid;
                if (!createdRecord) {
                    int error = Marshal.GetLastWin32Error(); record.Dispose();
                    if (error != 80 && error != 183) throw new Win32Exception(error);
                    record = CreateFileW(recordPath, GenericReadWrite | ReadControl, ShareRead,
                        IntPtr.Zero, OpenExisting, OpenReparsePoint | WriteThrough, IntPtr.Zero);
                }
            } else {
                record = CreateFileW(recordPath, ReadControl | ReadAttributes, ShareRead, IntPtr.Zero,
                    OpenExisting, OpenReparsePoint, IntPtr.Zero);
            }
            if (record.IsInvalid) {
                int error = Marshal.GetLastWin32Error(); record.Dispose();
                if (action == "pending" && error == 2) return new LmmMagpieJournalResult { Exists = false };
                throw new Win32Exception(error);
            }
            using (record) {
                FileInformation info = Information(record, false);
                CheckSecurity(record, sid, false);
                Need(info.Links == 1 && info.SizeHigh == 0 && info.SizeLow < 256);
                if (action == "pending") return new LmmMagpieJournalResult { Exists = true };
                using (FileStream stream = new FileStream(record, FileAccess.ReadWrite)) {
                    if (createdRecord) stream.Write(supplied, 0, supplied.Length);
                    // Windows supports flushing file data, not the POSIX directory
                    // fsync contract. Existing markers are also flushed on recovery.
                    stream.Flush(true);
                    Need(stream.Length > 0 && stream.Length < 256);
                    stream.Position = 0;
                    byte[] bytes = new byte[(int)stream.Length];
                    int read = 0;
                    while (read < bytes.Length) {
                        int count = stream.Read(bytes, read, bytes.Length - read);
                        Need(count > 0); read += count;
                    }
                    return new LmmMagpieJournalResult { Exists = true, Content = utf8.GetString(bytes) };
                }
            }
        } finally {
            for (int i = parents.Count - 1; i >= 0; i--) parents[i].Dispose();
            Marshal.FreeHGlobal(directorySecurity.Descriptor);
            Marshal.FreeHGlobal(fileSecurity.Descriptor);
        }
    }
}
'@

$stage = 'input'
try {
    $raw = [Console]::In.ReadToEnd()
    if ($raw.Length -gt 16384) { throw 'Invalid input.' }
    $inputData = ConvertFrom-Json -InputObject $raw
    Write-JournalPhase 'input_ready'
    $stage = 'compile'
    Write-JournalPhase 'compile_begin'
    Add-Type -TypeDefinition $source
    Write-JournalPhase 'compile_end'
    $stage = 'storage'
    Write-JournalPhase 'storage_begin'
    $result = [LmmMagpieJournalWindows]::Run(
        [string]$inputData.directory, [string]$inputData.name,
        [string]$inputData.action, $inputData.content)
    Write-JournalPhase 'storage_end'
    $stage = 'result'
    Write-JournalPhase 'result_ready'
    @{ exists = $result.Exists; content = $result.Content } | ConvertTo-Json -Compress
} catch {
    # Paths, SIDs, record contents and arbitrary native errors stay out of logs.
    $nativeCode = 0
    $failure = $_.Exception.GetBaseException()
    if ($failure -is [System.ComponentModel.Win32Exception]) { $nativeCode = $failure.NativeErrorCode }
    @{ error = 'unverified_windows_journal'; stage = $stage; nativeCode = $nativeCode } | ConvertTo-Json -Compress
    exit 1
}
