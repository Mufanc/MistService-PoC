package xyz.mufanc.ash

import android.os.Binder
import android.os.IServiceManager
import android.os.Looper
import android.os.Parcel
import android.os.RemoteException
import android.os.ServiceManager
import org.joor.Reflect
import xyz.mufanc.aproc.annotation.AProcEntry

@AProcEntry
object Main {

    private const val DUMP_FLAG_PRIORITY_HIDE = 1 shl 24

    private val sService1 = object : Binder() {
        override fun getInterfaceDescriptor(): String {
            return "com.example.IMistService1"
        }

        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            println("[Service1] onTransact: $code")
            return super.onTransact(code, data, reply, flags)
        }
    }

    private val sService2 = object : Binder() {
        override fun getInterfaceDescriptor(): String {
            return "com.example.IMistService2"
        }

        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            println("[Service2] onTransact: $code")
            return super.onTransact(code, data, reply, flags)
        }
    }

    @JvmStatic
    fun main(args: Array<String>) {
        if (args.isNotEmpty() && args[0] == "list") {
            val services = listServices(IServiceManager.DUMP_FLAG_PRIORITY_ALL or DUMP_FLAG_PRIORITY_HIDE)
            println("Found ${services.size} services:")
            services.forEachIndexed { index, service ->
                println("${index + 1}\t$service")
            }
        } else {
            runServices()
        }
    }

    @Suppress("DEPRECATION")
    private fun runServices() {
        ServiceManager.addService("mist_service_1", sService1)
        ServiceManager.addService("mist_service_2", sService2, false, DUMP_FLAG_PRIORITY_HIDE)

        Looper.prepareMainLooper()
        Looper.loop()
    }

    @Suppress("SameParameterValue")
    private fun listServices(dumpPriority: Int): Array<String> {
        return try {
            val ism = Reflect.onClass(ServiceManager::class.java).call("getIServiceManager").get<Any>()
            Reflect.on(ism).call("listServices", dumpPriority).get()
        } catch (err: RemoteException) {
            err.printStackTrace()
            emptyArray()
        }
    }
}
