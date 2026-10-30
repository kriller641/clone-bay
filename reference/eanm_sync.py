#!/usr/bin/env python3
"""
eanm_sync.py - Sync EVE Online UI settings across characters/accounts,
while every account keeps its own module rack layout.

EVE keeps UI settings in the client's settings_* profile folder:
    core_char_<characterID>.dat   per-character settings (window layout etc.)
    core_user_<accountID>.dat     per-account settings (overview, probes, ...)

This script copies one file's settings onto every other file of the same
kind, EXCEPT for these keys in the 'ui' section, which each target keeps
exactly as it was:

    slotOrder         module rack layout on the HUD, stored per individual
                      ship (by item ID) as 24 slot flags in display order
                      (11-18 low, 19-26 mid, 27-34 high slots)
    linkedWeapons_*   weapon groups

A target with none stored ends up with none - layouts are never merged. These
keys live in core_user files; core_char files are checked for them too.

When neither the source nor the target holds any of these keys, the source
file is copied byte for byte, exactly like a plain copy. Otherwise the file
is decoded, the target's keys are swapped back in, and it is re-encoded in
the client's blue.marshal format. That is the same output EVE Wrench writes,
and it is about 40% larger than the client's own because shared references
are written out in full.

Safety:
  * will not run while EVE (exefile.exe) is running
  * checks every file before touching anything (use --dry-run to see what
    it would do without changing anything)
  * backs up every target into eanm_sync_backup_<time>/ next to the
    settings files and verifies the backups before writing anything
  * every re-encoded file is decoded again and compared with the source and
    target before it is written; a file that can't be handled exactly is
    skipped and left untouched
  * files are replaced atomically; a target that is read-only, or changed
    after it was checked, is left untouched

To undo: with EVE closed, copy the files from the backup folder back into the
settings folder.

Usage:
    python eanm_sync.py                     # current folder, asks what to do
    python eanm_sync.py "C:\\path\\to\\settings_Default"
    python eanm_sync.py --chars             # only sync character settings
    python eanm_sync.py --users             # only sync account settings
    python eanm_sync.py --dry-run           # show the plan, change nothing

Needs Python 3.7 or newer; standard library only.
"""

import argparse
import datetime
import os
import re
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

# 'ui' keys (prefix match) that every target keeps as it was. Add
# "showEmptySlots" to also keep the HUD's "show empty slots" toggle.
PROTECTED_UI_PREFIXES = ("slotOrder", "linkedWeapons_")

CHAR_RE = re.compile(r"^core_char_(\d+)\.dat$", re.IGNORECASE)
USER_RE = re.compile(r"^core_user_(\d+)\.dat$", re.IGNORECASE)

# Marshal's built-in string table (MARSHAL_STRINGS in CCP's Marshal.cpp)
STR_TABLE = [
    '*corpid',
    '*locationid',
    'age',
    'Asteroid',
    'authentication',
    'ballID',
    'beyonce',
    'bloodlineID',
    'capacity',
    'categoryID',
    'character',
    'characterID',
    'characterName',
    'characterType',
    'charID',
    'chatx',
    'clientID',
    'config',
    'contraband',
    'corporationDateTime',
    'corporationID',
    'createDateTime',
    'customInfo',
    'description',
    'divisionID',
    'DoDestinyUpdate',
    'dogmaIM',
    'EVE System',
    'flag',
    'foo.SlimItem',
    'gangID',
    'Gemini',
    'gender',
    'graphicID',
    'groupID',
    'header',
    'idName',
    'invbroker',
    'itemID',
    'items',
    'jumps',
    'line',
    'lines',
    'locationID',
    'locationName',
    'macho.CallReq',
    'macho.CallRsp',
    'macho.MachoAddress',
    'macho.Notification',
    'macho.SessionChangeNotification',
    'modules',
    'name',
    'objectCaching',
    'objectCaching.CachedObject',
    'OnChatJoin',
    'OnChatLeave',
    'OnChatSpeak',
    'OnGodmaShipEffect',
    'OnItemChange',
    'OnModuleAttributeChange',
    'OnMultiEvent',
    'orbitID',
    'ownerID',
    'ownerName',
    'quantity',
    'raceID',
    'RowClass',
    'securityStatus',
    'Sentry Gun',
    'sessionchange',
    'singleton',
    'skillEffect',
    'squadronID',
    'typeID',
    'used',
    'userID',
    'util.CachedObject',
    'util.IndexRowset',
    'util.Moniker',
    'util.Row',
    'util.Rowset',
    '*multicastID',
    'AddBalls',
    'AttackHit3',
    'AttackHit3R',
    'AttackHit4R',
    'DoDestinyUpdates',
    'GetLocationsEx',
    'InvalidateCachedObjects',
    'JoinChannel',
    'LSC',
    'LaunchMissile',
    'LeaveChannel',
    'OID+',
    'OID-',
    'OnAggressionChange',
    'OnCharGangChange',
    'OnCharNoLongerInStation',
    'OnCharNowInStation',
    'OnDamageMessage',
    'OnDamageStateChange',
    'OnEffectHit',
    'OnGangDamageStateChange',
    'OnLSC',
    'OnSpecialFX',
    'OnTarget',
    'RemoveBalls',
    'SendMessage',
    'SetMaxSpeed',
    'SetSpeedFraction',
    'TerminalExplosion',
    'address',
    'alert',
    'allianceID',
    'allianceid',
    'bid',
    'bookmark',
    'bounty',
    'channel',
    'charid',
    'constellationid',
    'corpID',
    'corpid',
    'corprole',
    'damage',
    'duration',
    'effects.Laser',
    'gangid',
    'gangrole',
    'hqID',
    'issued',
    'jit',
    'languageID',
    'locationid',
    'machoVersion',
    'marketProxy',
    'minVolume',
    'orderID',
    'price',
    'range',
    'regionID',
    'regionid',
    'role',
    'rolesAtAll',
    'rolesAtBase',
    'rolesAtHQ',
    'rolesAtOther',
    'shipid',
    'sn',
    'solarSystemID',
    'solarsystemid',
    'solarsystemid2',
    'source',
    'splash',
    'stationID',
    'stationid',
    'target',
    'userType',
    'userid',
    'volEntered',
    'volRemaining',
    'weapon',
    'agent.missionTemplatizedContent_BasicKillMission',
    'agent.missionTemplatizedContent_ResearchKillMission',
    'agent.missionTemplatizedContent_StorylineKillMission',
    'agent.missionTemplatizedContent_GenericStorylineKillMission',
    'agent.missionTemplatizedContent_BasicCourierMission',
    'agent.missionTemplatizedContent_ResearchCourierMission',
    'agent.missionTemplatizedContent_StorylineCourierMission',
    'agent.missionTemplatizedContent_GenericStorylineCourierMission',
    'agent.missionTemplatizedContent_BasicTradeMission',
    'agent.missionTemplatizedContent_ResearchTradeMission',
    'agent.missionTemplatizedContent_StorylineTradeMission',
    'agent.missionTemplatizedContent_GenericStorylineTradeMission',
    'agent.offerTemplatizedContent_BasicExchangeOffer',
    'agent.offerTemplatizedContent_BasicExchangeOffer_ContrabandDemand',
    'agent.offerTemplatizedContent_BasicExchangeOffer_Crafting',
    'agent.LoyaltyPoints',
    'agent.ResearchPoints',
    'agent.Credits',
    'agent.Item',
    'agent.Entity',
    'agent.Objective',
    'agent.FetchObjective',
    'agent.EncounterObjective',
    'agent.DungeonObjective',
    'agent.TransportObjective',
    'agent.Reward',
    'agent.TimeBonusReward',
    'agent.MissionReferral',
    'agent.Location',
    'agent.StandardMissionDetails',
    'agent.OfferDetails',
    'agent.ResearchMissionDetails',
    'agent.StorylineMissionDetails',
]

# ---------------------------------------------------------------------------
# blue.marshal codec
#
# A port of the blue-marshal crate that EVE Wrench uses, itself a port of
# Marshal.cpp from CCP's carbonengine/blue (the client's own reader). The
# decoder is deliberately stricter than both: anything unusual raises
# MarshalError and the file is left alone, rather than being guessed at.
#
# Encoding writes format version 1 with no shared references - the same
# output EVE Wrench produces, and a form the client's reader accepts natively
# (Marshal.cpp reads TY_SIGNATURE2 streams, and reads TY_BUFFER and TY_UTF8 as
# ordinary strings; the client's own writer uses TY_BUFFER for strings).
# ---------------------------------------------------------------------------

class MarshalError(Exception):
    pass


class Long(int):
    """Python 2 'long' (TY_LONG), kept distinct from plain int on re-encode."""


class Tuple(list):
    """A marshal tuple (a list subclass so it can hold anything)."""


class Dict(list):
    """Order-preserving dict as a list of (key, value) pairs."""


class Global(str):
    """TY_GLOBAL: a class/function referenced by dotted name."""


class Instance:
    def __init__(self, cls, state):
        self.cls, self.state = cls, state


class Reduce:
    def __init__(self, newobj, callable_, args, state, list_items, dict_items):
        self.newobj, self.callable, self.args = newobj, callable_, args
        self.state, self.list_items, self.dict_items = state, list_items, dict_items


class Callback:
    def __init__(self, inner):
        self.inner = inner


_MISSING = object()          # a Reduce with no state (distinct from state None)

TY_SHAREDFLAG, TY_TYPEMASK = 0x40, 0x3F

# The client allows 1000 levels; settings files use about a dozen. Kept low so
# Python's own recursion limit is never reached (each level is a few frames).
MAX_DEPTH = 100


class _Reader:
    def __init__(self, buf):
        self.buf = buf
        self.pos = 0
        self.ssize = len(buf)       # content end (v0 excludes the shared map)
        self.version = 0
        self.shared = []
        self.mapping = []
        self.num_shared_seen = 0
        self.depth = 0
        self.got_crc = False
        self.crc_pos = 0
        self.crc_val = 0
        self.str_table_uses = 0

    def take(self, n):
        if n < 0 or self.pos + n > self.ssize:
            raise MarshalError("unexpected end of data")
        s = self.buf[self.pos:self.pos + n]
        self.pos += n
        return s

    def u8(self):
        return self.take(1)[0]

    def peek(self):
        if self.pos >= self.ssize:
            raise MarshalError("unexpected end of data")
        return self.buf[self.pos]

    def unpack(self, fmt, n):
        return struct.unpack("<" + fmt, self.take(n))[0]

    def integer(self):
        c = self.u8()
        return self.unpack("i", 4) if c == 0xFF else c

    def count(self):
        n = self.integer()
        if n < 0:
            raise MarshalError("negative length in stream")
        return n

    def buff(self):
        return self.take(self.count())

    def header(self):
        tag = self.u8()
        if tag == 126:
            self.version = 0
        elif tag == 125:
            self.version = self.u8()
        else:
            raise MarshalError("not a marshal stream (bad signature)")
        if self.version == 0:
            mapcount = self.unpack("i", 4)
            if mapcount < 0:
                raise MarshalError("negative shared map count")
            if mapcount:
                start = len(self.buf) - mapcount * 4
                if start < self.pos:
                    raise MarshalError("truncated shared-object map")
                self.mapping = list(struct.unpack_from("<%di" % mapcount, self.buf, start))
                if any(v < 1 or v > mapcount for v in self.mapping):
                    raise MarshalError("bogus shared-object map entry")
                self.ssize = start
                self.shared = [None] * mapcount

    # Shared objects: a slot is allocated when the object starts (before its
    # children, matching Marshal.cpp) and filled once it is complete. Slot
    # values are never None for a finished object, so None means "not ready".
    def alloc(self):
        if self.version == 0:
            if self.num_shared_seen >= len(self.mapping):
                raise MarshalError("shared object table overflow")
            ix = self.mapping[self.num_shared_seen] - 1
            self.num_shared_seen += 1
            return ix
        self.shared.append(None)
        return len(self.shared) - 1

    def store(self, shared, value):
        if shared:
            self.shared[self.alloc()] = value
        return value

    def container(self, shared, build):
        ix = self.alloc() if shared else None
        value = build()
        if ix is not None:
            self.shared[ix] = value
        return value

    def reference(self):
        ref = self.integer()
        ix = ref - 1 if self.version == 0 else ref
        if not 0 <= ix < len(self.shared):
            raise MarshalError("invalid shared reference %d" % ref)
        value = self.shared[ix]
        if value is None:
            # The client either rejects this or builds a self-referencing
            # structure, which can't be represented faithfully here
            raise MarshalError("reference to an object that isn't complete yet")
        return value

    def verify_crc(self, end):
        end = min(end, len(self.buf))
        if zlib.adler32(self.buf[self.crc_pos:end], 1) != (self.crc_val & 0xFFFFFFFF):
            raise MarshalError("checksum mismatch (file damaged?)")

    def obj(self):
        self.depth += 1
        if self.depth > MAX_DEPTH:
            raise MarshalError("data nested too deeply")
        try:
            return self._obj()
        finally:
            self.depth -= 1

    def _obj(self):
        raw = self.u8()
        shared = bool(raw & TY_SHAREDFLAG)
        t = raw & TY_TYPEMASK
        if t == 1: return None
        if t == 31: return True
        if t == 32: return False
        if t == 7: return -1
        if t == 8: return 0
        if t == 9: return 1
        if t == 6: return self.unpack("b", 1)
        if t == 5: return self.unpack("h", 2)
        if t == 4: return self.unpack("i", 4)
        if t == 3: return self.unpack("q", 8)
        if t == 11: return 0.0
        if t == 10: return self.unpack("d", 8)
        if t == 47:
            b = self.buff()
            return self.store(shared, Long(int.from_bytes(b, "little", signed=True) if b else 0))
        if t == 14: return b""
        if t == 15: return self.take(1)
        if t == 16: return self.take(self.u8())
        if t == 17:
            i = self.u8()
            if not 1 <= i <= len(STR_TABLE):
                raise MarshalError("bad string table index %d" % i)
            self.str_table_uses += 1
            return STR_TABLE[i - 1].encode()
        if t in (13, 19): return self.store(shared, self.buff())
        if t == 40: return ""
        if t == 41: return self.take(2).decode("utf-16-le")
        if t == 18: return self.take(self.count() * 2).decode("utf-16-le")
        if t == 46: return self.buff().decode("utf-8")
        if t == 2: return self.store(shared, Global(self.buff().decode("utf-8")))
        if t == 36: return Tuple()
        if t == 37: return self.container(shared, lambda: Tuple([self.obj()]))
        if t == 44: return self.container(shared, lambda: Tuple([self.obj(), self.obj()]))
        if t == 20:
            n = self.count()
            return self.container(shared, lambda: Tuple([self.obj() for _ in range(n)]))
        if t == 38: return []
        if t == 39: return self.container(shared, lambda: [self.obj()])
        if t == 21:
            n = self.count()
            return self.container(shared, lambda: [self.obj() for _ in range(n)])
        if t == 22:
            n = self.count()

            def build():
                pairs = Dict()
                for _ in range(n):
                    v = self.obj()      # value comes before key on the wire
                    k = self.obj()
                    pairs.append((k, v))
                return pairs
            return self.container(shared, build)
        if t == 23:
            def build():
                guid = self.obj()
                if isinstance(guid, bytes):
                    guid = guid.decode("utf-8")     # strict: never alter a name
                if not isinstance(guid, str):
                    raise MarshalError("instance class name isn't a string")
                return Instance(guid, self.obj())
            return self.container(shared, build)
        if t in (34, 35):
            newobj = t == 35

            def build():
                rv = self.obj()
                if not isinstance(rv, Tuple):
                    raise MarshalError("reduce payload isn't a tuple")
                if newobj:
                    if not rv:
                        raise MarshalError("empty newobj tuple")
                    callable_, args, state = None, rv[0], (rv[1] if len(rv) > 1 else _MISSING)
                else:
                    if len(rv) < 2:
                        raise MarshalError("reduce tuple too short")
                    callable_, args, state = rv[0], rv[1], (rv[2] if len(rv) > 2 else _MISSING)
                items = []
                while self.peek() != 45:      # exact, as Marshal.cpp checks it
                    items.append(self.obj())
                self.pos += 1
                ditems = []
                while self.peek() != 45:      # exact, as Marshal.cpp checks it
                    ditems.append((self.obj(), self.obj()))
                self.pos += 1
                return Reduce(newobj, callable_, args, state, items, ditems)
            return self.container(shared, build)
        if t == 25: return Callback(self.obj())
        if t == 27: return self.reference()
        if t == 28:
            self.crc_val = self.unpack("i", 4)
            self.got_crc = True
            self.crc_pos = self.pos
            if self.version == 0:
                self.verify_crc(len(self.buf))
            return self.obj()
        raise MarshalError("unsupported marshal type tag %d" % t)


class Decoded:
    """Result of marshal_decode: the value plus facts about the stream."""

    def __init__(self, value, reader):
        self.value = value
        self.had_crc = reader.got_crc
        self.version = reader.version
        self.trailing = reader.ssize - reader.pos       # unread content bytes
        self.unused_shared = len(reader.mapping) - reader.num_shared_seen
        self.str_table_uses = reader.str_table_uses


def marshal_decode(buf):
    r = _Reader(bytes(buf))
    try:
        r.header()
        value = r.obj()
        if r.got_crc and r.version > 0:
            r.verify_crc(r.pos)
    except UnicodeDecodeError as e:
        raise MarshalError("invalid text in stream (%s)" % e.reason)
    return Decoded(value, r)


class _Writer:
    def __init__(self):
        self.out = bytearray()
        self.depth = 0

    def integer(self, n):
        if 0 <= n < 255:
            self.out.append(n)
        else:
            self.out.append(0xFF)
            self.out += struct.pack("<i", n)

    def buff(self, b):
        self.integer(len(b))
        self.out += b

    def bytes_(self, b):
        if not b:
            self.out.append(14)
        elif len(b) == 1:
            self.out.append(15)
            self.out += b
        else:
            self.out.append(19)
            self.buff(b)

    def seq(self, items, t0, t1, tn, t2=None):
        n = len(items)
        if n == 0:
            self.out.append(t0)
            return
        if n == 1:
            self.out.append(t1)
        elif n == 2 and t2 is not None:
            self.out.append(t2)
        else:
            self.out.append(tn)
            self.integer(n)
        for it in items:
            self.value(it)

    def value(self, v):
        self.depth += 1
        if self.depth > MAX_DEPTH:
            raise MarshalError("data nested too deeply")
        try:
            self._value(v)
        finally:
            self.depth -= 1

    def _value(self, v):
        o = self.out
        if v is None:
            o.append(1)
        elif v is True:
            o.append(31)
        elif v is False:
            o.append(32)
        elif isinstance(v, Long):
            o.append(47)
            n = int(v)
            if n == 0:
                self.buff(b"")
            else:
                b = n.to_bytes((n.bit_length() + 8) // 8, "little", signed=True)
                # minimal two's complement, as num-bigint writes it
                while len(b) > 1 and ((b[-1] == 0 and b[-2] < 0x80) or (b[-1] == 0xFF and b[-2] >= 0x80)):
                    b = b[:-1]
                self.buff(b)
        elif isinstance(v, int):
            if v == -1: o.append(7)
            elif v == 0: o.append(8)
            elif v == 1: o.append(9)
            elif -128 <= v <= 127: o.append(6); o += struct.pack("<b", v)
            elif -32768 <= v <= 32767: o.append(5); o += struct.pack("<h", v)
            elif -2**31 <= v < 2**31: o.append(4); o += struct.pack("<i", v)
            elif -2**63 <= v < 2**63: o.append(3); o += struct.pack("<q", v)
            else: raise MarshalError("integer out of range")
        elif isinstance(v, float):
            if v == 0.0:
                o.append(11)
            else:
                o.append(10)
                o += struct.pack("<d", v)
        elif isinstance(v, (bytes, bytearray)):
            self.bytes_(bytes(v))
        elif isinstance(v, Global):
            o.append(2)
            self.buff(v.encode("utf-8"))
        elif isinstance(v, str):
            if not v:
                o.append(40)
            else:
                o.append(46)
                self.buff(v.encode("utf-8"))
        elif isinstance(v, Tuple):
            self.seq(v, 36, 37, 20, t2=44)
        elif isinstance(v, Dict):
            o.append(22)
            self.integer(len(v))
            for k, val in v:
                self.value(val)
                self.value(k)
        elif isinstance(v, list):
            self.seq(v, 38, 39, 21)
        elif isinstance(v, Instance):
            o.append(23)
            self.bytes_(v.cls.encode("utf-8"))
            self.value(v.state)
        elif isinstance(v, Reduce):
            o.append(35 if v.newobj else 34)
            head = [v.args] if v.newobj else [v.callable, v.args]
            if v.state is not _MISSING:
                head.append(v.state)
            self.value(Tuple(head))
            for it in v.list_items:
                self.value(it)
            o.append(45)
            for k, val in v.dict_items:
                self.value(k)
                self.value(val)
            o.append(45)
        elif isinstance(v, Callback):
            o.append(25)
            self.value(v.inner)
        else:
            raise MarshalError("can't encode %r" % type(v))


def marshal_encode(value, checksum):
    w = _Writer()
    w.out += b"\x7d\x01"                     # TY_SIGNATURE2, version 1
    crc_at = None
    if checksum:
        w.out.append(28)
        crc_at = len(w.out)
        w.out += b"\0\0\0\0"
    w.value(value)
    if crc_at is not None:
        w.out[crc_at:crc_at + 4] = struct.pack("<I", zlib.adler32(bytes(w.out[crc_at + 4:]), 1))
    return bytes(w.out)


# ---------------------------------------------------------------------------
# Settings files
# ---------------------------------------------------------------------------

class SettingsError(Exception):
    """Why a file is being left alone."""


def key_name(key):
    if isinstance(key, bytes):
        return key.decode("utf-8", "replace")
    if isinstance(key, str):
        return key
    return None


def is_protected(key):
    name = key_name(key)
    return name is not None and name.startswith(PROTECTED_UI_PREFIXES)


def find_ui(root):
    """Index of the top-level 'ui' entry, or None. Refuses odd layouts."""
    found = [i for i, (k, _) in enumerate(root) if key_name(k) == "ui"]
    if len(found) > 1:
        raise SettingsError("file has more than one 'ui' section")
    if found and not isinstance(root[found[0]][1], Dict):
        raise SettingsError("'ui' section has an unexpected structure")
    return found[0] if found else None


def protected_pairs(root):
    i = find_ui(root)
    return [] if i is None else [(k, v) for k, v in root[i][1] if is_protected(k)]


class SettingsFile:
    """A decoded core_*.dat. Raises SettingsError for anything unexpected."""

    def __init__(self, path):
        self.path = path
        try:
            self.data = path.read_bytes()
        except OSError as e:
            raise SettingsError("can't read it (%s)" % (e.strerror or e))
        try:
            d = marshal_decode(self.data)
        except MarshalError as e:
            raise SettingsError("can't decode it (%s)" % e)
        except RecursionError:
            raise SettingsError("can't decode it (data nested too deeply)")
        if not isinstance(d.value, Dict):
            raise SettingsError("unexpected structure (top level isn't a dict)")
        if d.trailing or d.unused_shared:
            raise SettingsError("unexpected extra data in the file")
        self.root = d.value
        self.had_crc = d.had_crc
        self.protected = protected_pairs(self.root)


def build_protected_copy(source_root, target_root):
    """The source's settings with the target's protected 'ui' keys in place
    of the source's own. Inputs are not modified."""
    keep = protected_pairs(target_root)
    result = Dict(source_root)
    i = find_ui(result)
    if i is None:
        if keep:
            result.append((b"ui", Dict(keep)))
        return result
    key, src_ui = result[i]
    result[i] = (key, Dict([(k, v) for k, v in src_ui if not is_protected(k)] + keep))
    return result


def _canon(v):
    return marshal_encode(v, False)


def _split(root):
    """Canonical bytes of (other top-level entries, unprotected ui entries,
    protected ui entries), each in order."""
    i = find_ui(root)
    other = [(_canon(k), _canon(v)) for j, (k, v) in enumerate(root) if j != i]
    ui = root[i][1] if i is not None else Dict()
    rest = [(_canon(k), _canon(v)) for k, v in ui if not is_protected(k)]
    prot = [(_canon(k), _canon(v)) for k, v in ui if is_protected(k)]
    return other, rest, prot


def verify_output(data, source_root, target_root):
    """Decode what is about to be written and prove it holds exactly the
    source's settings plus the target's protected keys."""
    try:
        out = marshal_decode(data)
    except MarshalError as e:
        raise SettingsError("self-check failed: output doesn't decode (%s)" % e)
    if not isinstance(out.value, Dict) or out.trailing:
        raise SettingsError("self-check failed: output has the wrong structure")
    o_other, o_rest, o_prot = _split(out.value)
    s_other, s_rest, _ = _split(source_root)
    _, _, t_prot = _split(target_root)
    if o_other != s_other or o_rest != s_rest:
        raise SettingsError("self-check failed: settings differ from the source")
    if o_prot != t_prot:
        raise SettingsError("self-check failed: protected keys differ from the target")


def describe(pairs):
    parts = []
    for k, v in pairs:
        name = key_name(k)
        inner = v[1] if isinstance(v, Tuple) and len(v) == 2 else v
        n = len(inner) if isinstance(inner, list) else None
        if name == "slotOrder" and n is not None:
            parts.append("module rack layout (%d ship%s)" % (n, "" if n == 1 else "s"))
        elif name == "linkedWeapons_groupsDict" and n is not None:
            parts.append("weapon groups (%d ship%s)" % (n, "" if n == 1 else "s"))
        elif n is not None:
            parts.append("%s (%d entries)" % (name, n))
        else:
            parts.append(name)
    return ", ".join(parts)


class Plan:
    def __init__(self, path, mode, note, data=None, original=None):
        self.path, self.mode, self.note = path, mode, note
        self.data, self.original = data, original


def plan_target(source, path):
    if not os.access(str(path), os.W_OK):
        return Plan(path, "skip", "SKIPPED: file is read-only")
    try:
        target = SettingsFile(path)
    except SettingsError as e:
        return Plan(path, "skip", "SKIPPED: %s" % e)
    if not source.protected and not target.protected:
        return Plan(path, "exact", "exact copy", source.data, target.data)
    try:
        result = build_protected_copy(source.root, target.root)
        data = marshal_encode(result, target.had_crc)
        verify_output(data, source.root, target.root)
    except (SettingsError, MarshalError, RecursionError) as e:
        return Plan(path, "skip", "SKIPPED: %s" % e)
    if target.protected:
        note = "copy, keeping its own: " + describe(target.protected)
    else:
        note = "copy; it has no rack layout or weapon groups stored, so it gets none"
    return Plan(path, "keep", note, data, target.data)


# ---------------------------------------------------------------------------
# Doing it
# ---------------------------------------------------------------------------

def eve_running():
    """True/False, or None if it couldn't be checked."""
    try:
        if os.name == "nt":
            cmd = ["tasklist", "/FI", "IMAGENAME eq exefile.exe", "/NH"]
        else:
            cmd = ["ps", "-A", "-o", "comm="]
        out = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                             timeout=20).stdout.decode("ascii", "replace")
    except (OSError, subprocess.SubprocessError):
        return None
    return "exefile" in out.lower()


def make_backup(plans, folder: Path):
    """Back up every target and verify each copy. Returns the folder; raises
    OSError/SettingsError on any problem (before anything is written)."""
    stamp = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
    backup_dir = folder / ("eanm_sync_backup_" + stamp)
    n = 1
    while backup_dir.exists():
        n += 1
        backup_dir = folder / ("eanm_sync_backup_%s_%d" % (stamp, n))
    backup_dir.mkdir()
    for p in plans:
        dest = backup_dir / p.path.name
        shutil.copy2(str(p.path), str(dest))
        if dest.read_bytes() != p.original:
            raise SettingsError("%s changed after it was checked, or its backup "
                                "doesn't match (is EVE running?)" % p.path.name)
    return backup_dir


def write_target(p):
    """Atomically replace one target. Returns an error string or None."""
    try:
        if p.path.read_bytes() != p.original:
            return "changed after it was checked (is EVE running?) - left untouched"
    except OSError as e:
        return "can't read it any more (%s) - left untouched" % (e.strerror or e)
    tmp = p.path.with_name(p.path.name + ".eanm_sync.tmp")
    try:
        with open(str(tmp), "wb") as f:
            f.write(p.data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(str(tmp), str(p.path))
    except OSError as e:
        try:
            tmp.unlink()
        except OSError:
            pass
        return "couldn't write it (%s) - left untouched" % (e.strerror or e)
    try:
        if p.path.read_bytes() == p.data:
            return None
    except OSError:
        pass
    return "WRITTEN BUT DOESN'T READ BACK CORRECTLY - restore it from the backup"


def ask(prompt):
    try:
        return input(prompt).strip()
    except EOFError:
        return ""


def sync(source_path: Path, bucket: list, folder: Path, label: str, dry_run: bool):
    targets = [p for p in bucket if p != source_path]
    if not targets:
        print("Nothing to overwrite (only one %s file)." % label)
        return True
    try:
        source = SettingsFile(source_path)
    except SettingsError as e:
        print("  Can't use %s as the source: %s. Nothing was changed." % (source_path.name, e))
        return False
    plans = [plan_target(source, p) for p in targets]

    if dry_run:
        print("\nCopy %s settings FROM %s to:" % (label, source_path.name))
        width = max(len(p.path.name) for p in plans)
        for p in plans:
            print("  %-*s  %s" % (width, p.path.name, p.note))
        print("Dry run: nothing was changed.")
        return True
    for p in plans:
        if p.mode == "skip":
            print("  %s: %s" % (p.path.name, p.note))
    todo = [p for p in plans if p.mode != "skip"]
    if not todo:
        print("Nothing can be written. Nothing was changed.")
        return False

    try:
        backup_dir = make_backup(todo, folder)
    except (OSError, SettingsError) as e:
        print("Backup failed (%s). Nothing was changed." % e)
        return False
    print("Backed up %d file(s) to %s" % (len(todo), backup_dir.name))

    ok = True
    for p in todo:
        err = write_target(p)
        if err:
            ok = False
            print("  %s: %s" % (p.path.name, err))
        else:
            print("  %s -> %s   done" % (source_path.name, p.path.name))
    return ok and len(todo) == len(plans)


# ---------------------------------------------------------------------------
# Folder handling
# ---------------------------------------------------------------------------

def classify(path: Path):
    """Return 'char', 'user', or None depending on filename pattern."""
    if CHAR_RE.match(path.name):
        return "char"
    if USER_RE.match(path.name):
        return "user"
    return None


def scan(folder: Path):
    """Find and bucket all settings files, newest-modified first."""
    chars, users = [], []
    for p in sorted(folder.iterdir()):
        if not p.is_file():
            continue
        kind = classify(p)
        if kind == "char":
            chars.append(p)
        elif kind == "user":
            users.append(p)
    chars.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    users.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return chars, users


def choose(files, label):
    if not files:
        print("No %s files found." % label)
        return None
    print("\n%s files (most recently modified first):" % label.capitalize())
    for i, p in enumerate(files):
        mtime = datetime.datetime.fromtimestamp(p.stat().st_mtime)
        print("  [%d] %s   (modified %s)" % (i, p.name, mtime.strftime("%Y-%m-%d %H:%M")))
    while True:
        raw = ask("Pick the %s file to copy FROM (Enter to skip): " % label)
        if raw == "":
            return None
        if raw.isdigit() and 0 <= int(raw) < len(files):
            return files[int(raw)]
        print("Invalid choice, try again.")


def main():
    if sys.version_info < (3, 7):
        sys.exit("This script needs Python 3.7 or newer.")
    ap = argparse.ArgumentParser(description="Sync EVE Online UI settings across characters/accounts, "
                                             "keeping each account's module rack layout.")
    ap.add_argument("folder", nargs="?", default=".", help="EVE settings_* folder (default: current directory)")
    ap.add_argument("--chars", action="store_true", help="Sync character-specific settings")
    ap.add_argument("--users", action="store_true", help="Sync account-specific settings")
    ap.add_argument("--dry-run", action="store_true", help="Check the files and show the plan, change nothing")
    args = ap.parse_args()

    folder = Path(args.folder).resolve()
    if not folder.is_dir():
        sys.exit("Not a folder: %s" % folder)

    chars, users = scan(folder)
    if not chars and not users:
        sys.exit(
            "No core_char_*.dat or core_user_*.dat files found here.\n"
            "Point this script at your EVE settings_Default folder, e.g.:\n"
            r'  python eanm_sync.py "%LOCALAPPDATA%\CCP\EVE\<install>\settings_Default"'
        )

    running = eve_running()
    if running:
        sys.exit("EVE is running (exefile.exe). Close every EVE client first - the client "
                 "saves its settings on logout and would overwrite the result.")
    if running is None:
        print("Note: couldn't check whether EVE is running. Make sure it's closed.")

    ok = True
    try:
        if args.chars or args.users:
            do_chars, do_users = args.chars, args.users
        else:
            do_chars = ask("Sync character-specific settings? [y/N] ").lower() == "y"
            do_users = ask("Sync account-specific settings? [y/N] ").lower() == "y"
        if do_chars:
            src = choose(chars, "character")
            if src:
                ok &= sync(src, chars, folder, "character", args.dry_run)
        if do_users:
            src = choose(users, "account/user")
            if src:
                ok &= sync(src, users, folder, "account", args.dry_run)
    except KeyboardInterrupt:
        print("\nInterrupted. Any file already written was backed up first.")
        sys.exit(1)

    print("\nDone." if ok else "\nDone, with problems (see above).")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
